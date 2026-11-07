"""Restricted declarative surface. This module cannot alter host validators."""
import hashlib
import json
from pathlib import Path

UPSTREAM_COMMIT = '6868817518876de8fd170c6a8354daca273b6191'
HOST_COMMIT = 'f3e49f0'  # Full commit is frozen in each experiment manifest.
GUARD = 'Use exact source quotes; preserve category and uncertainty. Never apply settings, modify sources, or change consent. Host validators and the 262144 context cap are immutable.\n'
BASE = {
    'systemprompt.md': json.dumps({'version':1, 'order':'filename', 'extraction':'逐文件保留连续引文及定位。', 'review':'保留限制与冲突，所有内容待人工核对。'}, ensure_ascii=False),
    'guardrails.md': GUARD,
    'LongTermMEMORY.md': '',
    'tool_descriptions/tool_guidance.md': 'Only the fixed host extraction, quotation validation and pack functions may run.\n',
}

def validate_tree(tree):
    if set(tree) != set(BASE):
        raise ValueError('Only the four declared text files are allowed; scripts, plugins and extra files are rejected')
    for key in ('guardrails.md','LongTermMEMORY.md','tool_descriptions/tool_guidance.md'):
        if tree[key] != BASE[key]:
            raise ValueError('Immutable host boundary: '+key)
    if not isinstance(tree['systemprompt.md'], str) or len(tree['systemprompt.md'].encode()) > 12000:
        raise ValueError('Strategy text too large')
    v=json.loads(tree['systemprompt.md'])
    if set(v)!={'version','order','extraction','review'} or type(v['version']) is not int or v['version']!=1:
        raise ValueError('Unknown policy schema or fields')
    if v['order'] not in ('filename','constraints_first'):
        raise ValueError('Unknown organization strategy')
    for key in ('extraction','review'):
        if not isinstance(v[key],str) or not 1<=len(v[key])<=2000:
            raise ValueError('Invalid declarative prompt')
    return v

def read_tree(root):
    root=Path(root)
    if root.is_symlink(): raise ValueError('Symlink policy directory')
    result={}
    for p in root.rglob('*'):
        if p.is_symlink(): raise ValueError('Symlink policy entry')
        if p.is_file(): result[p.relative_to(root).as_posix()]=p.read_text(encoding='utf-8')
    validate_tree(result)
    return result

def signature(tree):
    validate_tree(tree)
    return hashlib.sha256(json.dumps(tree,sort_keys=True,ensure_ascii=False).encode()).hexdigest()
