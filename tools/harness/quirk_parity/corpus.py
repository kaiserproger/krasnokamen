"""Append-only recorded vanilla corpus of the quirk/farm parity gate (owner D128).

Vanilla runs only at record time, twice per case: observable paths that differ between
the two recordings become wildcards. Every gate run replays Krasnokamen against the
recording without starting vanilla.

Layout under ``corpus_root()/<mc-version>/`` (outside Git: Mojang-derived observations
never enter the repository, ADR 0022):

    manifest.jsonl            append-only log; every line carries the SHA-256 of the line before it
    cases/<case>/v<NNN>.json  one immutable file per recorded version of a case
    manifests/                the whole manifest as it stood before each re-record

``corpus-lock.jsonl`` next to this file is the Git-tracked anchor: one line per case
version this tree replays, with the case file's SHA-256 and the hash of the scenario
that produced it. Replay fails closed when a locked version is missing or edited, when
a manifest line or the chain was altered, or when a lock line was dropped. Recording
appends to both files; a case only changes through ``re-record`` (new version with a
stated reason; the old version, its lock line and the old manifest stay).

The lock is also checked against the trusted Git baseline (``main``): every lock line
there must still be here, so dropping a newer version's line cannot roll a case back,
and a case whose replayed version differs from the baseline's must have reached it by
``re-record`` with a reason. Cases the baseline does not have yet are listed as added.
"""
from __future__ import annotations

import datetime
import fcntl
import hashlib
import json
import os
import shutil
import subprocess
from pathlib import Path
from typing import Any

from .vanilla import WORK_ROOT

LOCK = Path(__file__).with_name("corpus-lock.jsonl")
GENESIS = "0" * 64
FORMAT = 1


class CorpusError(RuntimeError):
    pass


def corpus_root(explicit: str | None = None) -> Path:
    """`--corpus`, else KRASNOKAMEN_ORACLE_CORPUS, else `<work>/oracle-corpus`."""
    return Path(explicit or os.environ.get("KRASNOKAMEN_ORACLE_CORPUS") or WORK_ROOT / "oracle-corpus")


def canonical(value: Any) -> bytes:
    return json.dumps(value, sort_keys=True, separators=(",", ":"), ensure_ascii=False).encode()


def sha256(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def scenario_sha256(scenario: dict) -> str:
    """Hash of everything both servers execute and the gate compares (prose excluded).

    Setup, phases, boxes, seed, compare rules, tolerances and self-checks are all inside:
    changing any of them, a tolerance included, needs an explicit re-record with a reason.
    """
    return sha256(canonical({key: value for key, value in scenario.items()
                             if key != "description" and not key.startswith("_")}))


def lock_line(mc_version: str, entry: dict) -> str:
    return canonical({"mc": mc_version, "case": entry["case"], "version": entry["version"],
                      "sha256": entry["sha256"], "scenario_sha256": entry["scenario_sha256"]}).decode()


def parse_lock(text: str, mc_version: str, source: str) -> list[dict]:
    rows = []
    for number, raw in enumerate(text.splitlines(), 1):
        if not raw.strip():
            continue
        try:
            row = json.loads(raw)
        except ValueError as error:
            raise CorpusError(f"{source}:{number}: not JSON: {error}") from error
        if set(row) != {"mc", "case", "version", "sha256", "scenario_sha256"}:
            raise CorpusError(f"{source}:{number}: unexpected lock fields {sorted(row)}")
        if row["mc"] == mc_version:
            rows.append(row)
    return rows


def read_lock(mc_version: str, path: Path = LOCK) -> list[dict]:
    return parse_lock(path.read_text(), mc_version, str(path)) if path.is_file() else []


def trusted_lock(mc_version: str, ref: str, repo: Path, path: Path = LOCK) -> list[dict]:
    """The lock as the trusted Git ref has it; a ref without the lock file has no locked cases."""
    rel = path.resolve().relative_to(repo.resolve()).as_posix()
    if subprocess.run(["git", "rev-parse", "--verify", "--quiet", f"{ref}^{{commit}}"], cwd=repo,
                      capture_output=True, check=False).returncode:
        raise CorpusError(f"trusted ref {ref!r} is not a commit of {repo}: name the baseline with --trusted-ref")
    shown = subprocess.run(["git", "show", f"{ref}:{rel}"], cwd=repo, capture_output=True, text=True, check=False)
    if shown.returncode == 0:
        return parse_lock(shown.stdout, mc_version, f"{ref}:{rel}")
    if "does not exist" in shown.stderr or "exists on disk, but not in" in shown.stderr:
        return []
    raise CorpusError(f"cannot read {ref}:{rel}: {shown.stderr.strip()}")


class Corpus:
    def __init__(self, root: Path, mc_version: str, lock: Path | None = None):
        self.mc_version = mc_version
        self.dir = root / mc_version
        self.lock = lock or LOCK
        self.manifest = self.dir / "manifest.jsonl"

    def _raw_lines(self) -> list[str]:
        if not self.manifest.is_file():
            return []
        return self.manifest.read_text().splitlines()

    def verify(self, trusted: list[dict] | None = None) -> dict[str, Any]:
        """Integrity of the manifest chain, every case file and the Git lock.

        Returns `latest` (case -> newest locked manifest entry), `problems` (any one fails
        the gate), `unanchored` (recorded by another tree, not in this tree's lock) and the
        manifest head hash and length. With `trusted` (the lock of the trusted Git baseline)
        it also returns `re_recorded` (cases replaying another version than the baseline, each
        reached by re-record with a reason) and `added` (cases the baseline does not have).
        """
        problems: list[str] = []
        entries: list[dict] = []
        prev = GENESIS
        for number, raw in enumerate(self._raw_lines(), 1):
            try:
                entry = json.loads(raw)
            except ValueError:
                problems.append(f"manifest line {number} is not JSON (manifest edited)")
                break
            if entry.get("prev") != prev or entry.get("seq") != number:
                problems.append(f"manifest line {number} ({entry.get('case')} v{entry.get('version')}): "
                                "hash chain broken (a manifest line was edited, removed or reordered)")
            prev = sha256(raw.encode())
            entries.append(entry)
        by_key: dict[tuple[str, int], dict] = {}
        for entry in entries:
            key = (entry.get("case"), entry.get("version"))
            if key in by_key:
                problems.append(f"manifest records {key[0]} v{key[1]} twice")
            by_key[key] = entry
            path = self.dir / str(entry.get("file"))
            if not path.is_file():
                problems.append(f"recorded case {key[0]} v{key[1]} removed: {path} is missing")
            elif sha256(path.read_bytes()) != entry.get("sha256"):
                problems.append(f"recorded case {key[0]} v{key[1]} edited: {path} no longer has "
                                f"SHA-256 {entry.get('sha256')}")
        locked: dict[str, list[dict]] = {}
        for row in read_lock(self.mc_version, self.lock):
            locked.setdefault(row["case"], []).append(row)
            entry = by_key.get((row["case"], row["version"]))
            if entry is None:
                problems.append(f"locked case {row['case']} v{row['version']} removed from the corpus manifest")
            elif entry["sha256"] != row["sha256"] or entry.get("scenario_sha256") != row["scenario_sha256"]:
                problems.append(f"locked case {row['case']} v{row['version']} edited: the corpus manifest "
                                "names another case file or scenario hash than the Git lock")
        latest = {}
        for case, rows in locked.items():
            versions = sorted(row["version"] for row in rows)
            if versions != list(range(1, len(versions) + 1)):
                problems.append(f"locked case {case} has versions {versions}: an older version's lock line "
                                "was removed")
            entry = by_key.get((case, versions[-1]))
            if entry is not None:
                latest[case] = entry
        anchored = {(row["case"], row["version"]) for rows in locked.values() for row in rows}
        state = {"latest": latest, "problems": problems, "manifest_head": prev, "entries": len(entries),
                 "unanchored": [entry for key, entry in by_key.items() if key not in anchored]}
        if trusted is not None:
            state.update(self._against_trusted(trusted, locked, by_key, problems))
        return state

    def _against_trusted(self, trusted: list[dict], locked: dict[str, list[dict]],
                         by_key: dict[tuple[str, int], dict], problems: list[str]) -> dict[str, Any]:
        """Lines of the trusted lock still present; version changes since it made by re-record."""
        here = {canonical(row) for rows in locked.values() for row in rows}
        baseline: dict[str, int] = {}
        for row in trusted:
            baseline[row["case"]] = max(baseline.get(row["case"], 0), row["version"])
            if canonical(row) not in here:
                problems.append(f"lock line of {row['case']} v{row['version']} on the trusted baseline is gone: "
                                "a recorded case only gains versions, it never rolls back")
        re_recorded = []
        for case, rows in sorted(locked.items()):
            newest = max(row["version"] for row in rows)
            if case not in baseline or newest == baseline[case]:
                continue
            steps = [by_key.get((case, version)) or {} for version in range(baseline[case] + 1, newest + 1)]
            if any(step.get("op") != "re-record" or not step.get("reason") for step in steps):
                problems.append(f"{case} replays v{newest} but the trusted baseline has v{baseline[case]}: "
                                "a newer version of a recorded case needs `re-record --reason`")
            re_recorded.append({"case": case, "from": baseline[case], "to": newest,
                                "reasons": [step.get("reason") for step in steps]})
        return {"re_recorded": re_recorded, "added": sorted(set(locked) - set(baseline))}

    def load(self, entry: dict) -> dict:
        data = (self.dir / entry["file"]).read_bytes()
        if sha256(data) != entry["sha256"]:
            raise CorpusError(f"recorded case {entry['case']} v{entry['version']} edited since verification")
        doc = json.loads(data)
        if doc.get("case") != entry["case"] or doc.get("version") != entry["version"]:
            raise CorpusError(f"{entry['file']} does not hold {entry['case']} v{entry['version']}")
        return doc

    def _locked(self):
        self.dir.mkdir(parents=True, exist_ok=True)
        handle = (self.dir / ".append.lock").open("w")
        fcntl.flock(handle, fcntl.LOCK_EX)
        return handle

    def append(self, doc: dict, *, op: str, reason: str | None = None) -> dict:
        """Store one new case version and append it to the manifest and the Git lock."""
        with self._locked():
            state = self.verify()
            if state["problems"]:
                raise CorpusError("refusing to append to a corpus that fails verification: "
                                  + "; ".join(state["problems"][:3]))
            rel = f"cases/{doc['case'].replace('#', '@')}/v{doc['version']:03d}.json"
            path = self.dir / rel
            path.parent.mkdir(parents=True, exist_ok=True)
            data = json.dumps(doc, indent=1, sort_keys=True).encode() + b"\n"
            with path.open("xb") as handle:  # case files are immutable: never overwrite
                handle.write(data)
            entry = {
                "seq": state["entries"] + 1, "op": op, "case": doc["case"], "version": doc["version"],
                "file": rel, "sha256": sha256(data), "scenario_sha256": doc["scenario_sha256"],
                "recorded_at": doc["recorded_at"], "reason": reason, "prev": state["manifest_head"],
            }
            with self.manifest.open("a") as handle:
                handle.write(canonical(entry).decode() + "\n")
            self.anchor(entry)
            return entry

    def anchor(self, entry: dict) -> None:
        """Append one case version to this tree's Git lock (recorded here or by another tree)."""
        with self.lock.open("a") as handle:
            handle.write(lock_line(self.mc_version, entry) + "\n")

    def keep_manifest(self, label: str) -> Path:
        """Copy of the manifest as it stands, kept before a re-record changes what replays."""
        stamp = datetime.datetime.now(datetime.timezone.utc).strftime("%Y%m%dT%H%M%SZ")
        target = self.dir / "manifests" / f"{stamp}-{label}.jsonl"
        target.parent.mkdir(parents=True, exist_ok=True)
        shutil.copyfile(self.manifest, target)
        return target
