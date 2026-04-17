#!/usr/bin/env python3
"""Migrate WhereClause metric fields to the metrics map in builtin JSON audit files."""
import json
from pathlib import Path

METRIC_FIELDS = {
    'cyclomatic_complexity', 'function_length', 'cognitive_complexity',
    'comment_to_code_ratio', 'efferent_coupling', 'afferent_coupling',
    'count', 'cycle_size', 'depth', 'edge_count', 'ratio',
}

def migrate_where_clause(wc):
    if not isinstance(wc, dict):
        return wc
    metrics = {}
    result = {}
    for key, value in wc.items():
        if key in METRIC_FIELDS:
            metrics[key] = value
        elif key == 'and':
            result[key] = [migrate_where_clause(c) for c in value]
        elif key == 'or':
            result[key] = [migrate_where_clause(c) for c in value]
        elif key == 'not':
            result[key] = migrate_where_clause(value)
        else:
            result[key] = value
    if metrics:
        result['metrics'] = metrics
    return result

def migrate_flag(flag):
    result = dict(flag)
    if 'severity_map' in result:
        new_map = []
        for entry in result['severity_map']:
            e = dict(entry)
            if 'when' in e and e['when'] is not None:
                e['when'] = migrate_where_clause(e['when'])
            new_map.append(e)
        result['severity_map'] = new_map
    return result

def migrate_ratio_config(ratio):
    result = dict(ratio)
    if 'threshold' in result and result['threshold'] is not None:
        result['threshold'] = migrate_where_clause(result['threshold'])
    if 'numerator' in result and isinstance(result['numerator'], dict):
        num = dict(result['numerator'])
        if 'where' in num and num['where'] is not None:
            num['where'] = migrate_where_clause(num['where'])
        result['numerator'] = num
    if 'denominator' in result and isinstance(result['denominator'], dict):
        den = dict(result['denominator'])
        if 'where' in den and den['where'] is not None:
            den['where'] = migrate_where_clause(den['where'])
        result['denominator'] = den
    return result

def migrate_stage(stage):
    result = {}
    for key, value in stage.items():
        if key == 'flag':
            result[key] = migrate_flag(value)
        elif key == 'ratio':
            result[key] = migrate_ratio_config(value)
        elif key == 'where':
            result[key] = migrate_where_clause(value)
        elif key == 'exclude':
            result[key] = migrate_where_clause(value)
        else:
            result[key] = value
    return result

def migrate_file(path):
    with open(path) as f:
        data = json.load(f)
    if 'graph' in data:
        data['graph'] = [migrate_stage(s) for s in data['graph']]
    with open(path, 'w') as f:
        json.dump(data, f, indent=2)
        f.write('\n')

if __name__ == '__main__':
    builtin_dir = Path('src/audit/builtin')
    files = sorted(builtin_dir.glob('*.json'))
    for p in files:
        migrate_file(p)
    print(f"Migrated {len(files)} files.")
