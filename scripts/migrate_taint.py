#!/usr/bin/env python3
"""Decompose monolithic taint stage into taint_sources + taint_sanitizers + taint_sinks."""
import json
from pathlib import Path

def migrate_taint_stage(stage):
    if 'taint' not in stage:
        return [stage]
    taint = stage['taint']
    result = []
    if taint.get('sources'):
        result.append({'taint_sources': taint['sources']})
    if taint.get('sanitizers'):
        result.append({'taint_sanitizers': taint['sanitizers']})
    if taint.get('sinks'):
        result.append({'taint_sinks': taint['sinks']})
    return result

def migrate_file(path):
    with open(path) as f:
        data = json.load(f)
    if 'graph' not in data:
        return False
    new_graph = []
    changed = False
    for stage in data['graph']:
        replacement = migrate_taint_stage(stage)
        new_graph.extend(replacement)
        if len(replacement) != 1 or replacement[0] is not stage:
            changed = True
    if changed:
        data['graph'] = new_graph
        with open(path, 'w') as f:
            json.dump(data, f, indent=2)
            f.write('\n')
    return changed

if __name__ == '__main__':
    builtin_dir = Path('src/audit/builtin')
    count = 0
    for p in sorted(builtin_dir.glob('*.json')):
        if migrate_file(p):
            count += 1
            print(f"  Migrated: {p.name}")
    print(f"\nMigrated {count} files.")
