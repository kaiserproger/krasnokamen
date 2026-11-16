"""Freeze and verify the complete SIGKILL evidence tree before inspection."""
import hashlib
import json
import shutil
from pathlib import Path


def manifest_bytes(files):
    return (json.dumps(files, indent=2, sort_keys=True) + '\n').encode()


def file_entry(path, relative):
    if path.is_symlink() or not path.is_file():
        raise ValueError(f'unsupported crash evidence: {path}')
    with path.open('rb') as source:
        digest = hashlib.file_digest(source, 'sha256').hexdigest()
    return {'path': relative, 'size': path.stat().st_size, 'sha256': digest}


def file_manifest(root):
    root = Path(root)
    result = []
    for path in sorted(root.rglob('*')):
        if path.is_symlink():
            raise ValueError(f'symlink in crash evidence: {path}')
        if path.is_file():
            result.append(file_entry(path, path.relative_to(root).as_posix()))
        elif not path.is_dir():
            raise ValueError(f'unsupported crash evidence: {path}')
    return sorted(result, key=lambda entry: entry['path'])


def source_manifest(root):
    """Hash stopped originals using the paths they will have in crash-copy."""
    root = Path(root)
    files = []
    for name in ('world', 'wal-archive', 'backups'):
        source = root / name
        if source.exists():
            if source.is_symlink() or not source.is_dir():
                raise ValueError(f'unsupported crash evidence: {source}')
            for entry in file_manifest(source):
                files.append({**entry, 'path': name + '/' + entry['path']})
    for name in ('server.log', 'before-census.json', 'acknowledged-inventory.json'):
        source = root / name
        if source.exists():
            relative = name if name == 'server.log' else f'witness/{name}'
            files.append(file_entry(source, relative))
    return sorted(files, key=lambda entry: entry['path'])


def verify_tree(destination, handoff):
    """Verify a derived tree and its separately retained pre-recovery manifest."""
    files = handoff['manifest']
    if handoff['original_manifest'] != files:
        raise ValueError('stopped original and crash copy manifest mismatch')
    encoded = manifest_bytes(files)
    expected_hash = handoff['manifest_sha256']
    manifest_path = Path(handoff['manifest_path'])
    if hashlib.sha256(encoded).hexdigest() != expected_hash or manifest_path.read_bytes() != encoded:
        raise ValueError('crash manifest hash or content mismatch')
    if Path(destination).resolve() == Path(handoff['crash_copy']).resolve():
        raise ValueError('refusing to inspect the primary crash copy')
    actual = file_manifest(destination)
    if actual != files:
        raise ValueError('crash copy does not match recorded manifest (missing, extra or changed file)')
    return {'status': 'verified', 'manifest_path': str(manifest_path),
            'manifest_sha256': expected_hash, 'copy': str(destination), 'files': len(files)}


def verified_copy(snapshot, destination, handoff):
    """Copy the primary evidence, then verify every file before any consumer opens it."""
    shutil.copytree(snapshot, destination, symlinks=True)
    return verify_tree(destination, handoff)
