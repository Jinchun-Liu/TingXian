"""No model/network: source-backed literal coverage proxy, never a semantic pass."""
import json
from pathlib import Path
import sys


def evaluate(label, bundle):
    records = [(doc['name'], record) for doc in bundle['documents'] for record in doc['records']]
    rows = []
    for atom in label['atoms']:
        sources = {atom['file'], *[s['file'] for s in atom.get('also_in', [])]}
        matches = [r for name, r in records if name in sources and atom['quote'] in r['quote']]
        rows.append({'atom': atom['id'], 'literal_covered': bool(matches),
                     'exact_category_matched': any(r['kind'] == atom['kind'] for r in matches)})
    return {'atoms': len(rows), 'literal_covered': sum(r['literal_covered'] for r in rows),
            'exact_category_matched': sum(r['exact_category_matched'] for r in rows), 'rows': rows,
            'semantic_accuracy': None, 'human_review': 'pending',
            'warning': 'Exact quote/category proxy only; does not detect hallucinated notes or validate profile/settings.'}


if __name__ == '__main__':
    config = json.loads(Path(sys.argv[1]).read_text())
    labels = {r['id']: r for r in map(json.loads, Path(config['labels']).read_text().splitlines())}
    rows = []
    for item in config['submissions']:
        try:
            rows.append({**item, 'status': 'scored_proxy', 'metrics': evaluate(labels[item['task']], json.loads(Path(item['path']).read_text()))})
        except Exception as exc:
            rows.append({**item, 'status': 'evaluation_error', 'error': type(exc).__name__})
    Path(config['output']).write_text(json.dumps(rows, ensure_ascii=False, indent=2))
