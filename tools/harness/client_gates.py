"""Strict client-repository gates that a `client-gate:<id>` ref runs.

A gate is one whole-repository command run in the client checkout (`KRASNOKAMEN_CLIENT_ROOT` or the
sibling checkout): a strict lint or format command whose failure is a real diagnostic, not a scene a
pinned client could show. Every id a row may cite is registered here; `ledger promote` refuses an
unregistered id by name, and `regressions --include-regression-client` (the client tier) runs the
registered ones. The default L2 regression run skips them like every other `client:` ref: they need
the client checkout and a client build, which L2 on the core tree does not do.

A row that proves a gate fix reverses the client commit that made the gate pass in a disposable copy
of the checkout (`fix_commit = "client-test:<sha>"`): the reversed tree has to make the gate exit
non-zero with the compiler's own diagnostic, never a missing tool or a build error.
"""

from __future__ import annotations

# id -> (the exact command line shown on the row, argv run in the client checkout)
GATES: dict[str, tuple[str, list[str]]] = {
    'clippy-workspace-all-targets': (
        'cargo clippy --workspace --all-targets -- -D warnings',
        ['cargo', 'clippy', '--workspace', '--all-targets', '--', '-D', 'warnings'],
    ),
    'clippy-client-all-targets': (
        'cargo clippy -p krasnokamen-client --all-targets -- -D warnings',
        ['cargo', 'clippy', '-p', 'krasnokamen-client', '--all-targets', '--', '-D', 'warnings'],
    ),
}


def command(gate_id: str) -> tuple[str, list[str]] | None:
    """(description, argv) of a registered gate, or None when the id is not registered."""
    return GATES.get(gate_id)


def ids() -> list[str]:
    """Every registered gate id, sorted."""
    return sorted(GATES)
