"""One-shot JSON/YAML field extraction from a byte snapshot on stdin.

No constructors outside SafeLoader, no file/network access, no partial output.
Values remain typed JSON literals so null, zero and empty strings stay distinct.
"""
import json
import math
import sys
from decimal import Decimal, InvalidOperation, localcontext

MAX_TEXT = 500_000
MAX_NODES = 20_000
MAX_DEPTH = 64


class NumberLiteral(str):
    """Keep JSON numeric spelling; never round source metrics via binary float."""


def yaml_decimal(loader, node):
    raw = node.value.replace('_', '').lower()
    try:
        if ':' in raw:
            sign = -1 if raw.startswith('-') else 1
            parts = raw.lstrip('+-').split(':')
            # YAML sexagesimal float parts are decimal digits, with a possible
            # fraction on the final part. Avoid rounding during base-60 sums.
            import re
            if any(not re.fullmatch(r'[0-9]+(?:\.[0-9]*)?', p) for p in parts):
                raise ValueError('YAML 数值格式无效')
            with localcontext() as context:
                context.prec = len(raw) + 10
                value = Decimal(0)
                for part in parts:
                    value = value * 60 + Decimal(part)
                value *= sign
        else:
            value = Decimal(raw)
        if not value.is_finite():
            raise ValueError('不支持无穷大或 NaN，请核对原始数值')
        return NumberLiteral(str(value))
    except InvalidOperation:
        raise ValueError('YAML 数值格式无效或非有限值，请核对原始数值') from None


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ValueError('JSON 含重复字段名，请先消除歧义')
        result[key] = value
    return result


def yaml_value(text):
    import yaml

    class BoundedLoader(yaml.SafeLoader):
        def __init__(self, stream):
            super().__init__(stream)
            self.depth = self.nodes = self.aliases = self.flattened = 0
            self.flattening = set()
            self.flatten_done = set()

        def compose_node(self, parent, index):
            self.nodes += 1
            self.depth += 1
            if self.check_event(yaml.AliasEvent):
                self.aliases += 1
            if self.depth > MAX_DEPTH or self.nodes > MAX_NODES or self.aliases > 64:
                raise ValueError('YAML 嵌套、节点或别名数量超限，未截断导入')
            try:
                return super().compose_node(parent, index)
            finally:
                self.depth -= 1

        def flatten_mapping(self, node):
            if id(node) in self.flattening:
                raise ValueError('YAML 含循环合并，无法形成有限资料字段')
            if id(node) in self.flatten_done:
                return
            seen = set()
            for key_node, _ in node.value:
                if key_node.tag == 'tag:yaml.org,2002:merge':
                    continue
                key = self.construct_object(key_node)
                if not isinstance(key, str):
                    raise ValueError('YAML 字段名必须是字符串；请为数字或布尔字段名加引号')
                if key in seen:
                    raise ValueError('YAML 含重复字段名，请先消除歧义')
                seen.add(key)
            self.flattening.add(id(node))
            try:
                super().flatten_mapping(node)
                self.flattened += len(node.value)
                if self.flattened > MAX_NODES:
                    raise ValueError('YAML 合并后的字段数量超限，未截断导入')
                self.flatten_done.add(id(node))
            finally:
                self.flattening.remove(id(node))

        def construct_mapping(self, node, deep=False):
            # Reject duplicate explicit keys; standard YAML merges may be
            # overridden by the explicit field in this mapping.
            self.flatten_mapping(node)
            return yaml.constructor.BaseConstructor.construct_mapping(self, node, deep=deep)

    # Keep date/time spellings as source text rather than inventing a timezone
    # or converting them to a non-JSON Python datetime object.
    BoundedLoader.add_constructor('tag:yaml.org,2002:timestamp', lambda loader, node: node.value)
    BoundedLoader.add_constructor('tag:yaml.org,2002:float', yaml_decimal)
    loader = BoundedLoader(text)
    try:
        return loader.get_single_data()
    except yaml.YAMLError as exc:
        mark = getattr(exc, 'problem_mark', None)
        location = f'（第 {mark.line + 1} 行）' if mark else ''
        raise ValueError(f'YAML 语法或标签不支持{location}；请使用单文档、普通映射/数组和标量') from None
    finally:
        loader.dispose()


def fields(value):
    output, active = [], set()
    text_count = 0

    def walk(item, pointer, depth):
        nonlocal text_count
        if depth > MAX_DEPTH or len(output) >= MAX_NODES:
            raise ValueError('结构化资料的嵌套或字段数量超限，未截断导入')
        container = isinstance(item, (dict, list))
        if container and id(item) in active:
            raise ValueError('YAML 含循环引用，无法形成有限资料字段')
        if container:
            active.add(id(item))
        if isinstance(item, dict):
            kind = 'object'
        elif isinstance(item, list):
            kind = 'array'
        elif item is None:
            kind = 'null'
        elif isinstance(item, bool):
            kind = 'boolean'
        elif isinstance(item, (NumberLiteral, int, float)):
            if isinstance(item, float) and not math.isfinite(item):
                raise ValueError('不支持无穷大或 NaN，请核对原始数值')
            kind = 'number'
        elif isinstance(item, str):
            kind = 'string'
        else:
            raise ValueError('不支持该 YAML 标量或集合类型，请转换成普通映射/数组和标量')
        literal = None if container and item else str(item) if isinstance(item, NumberLiteral) else json.dumps(item, ensure_ascii=False, allow_nan=False)
        text_count += len(pointer) + (len(literal) if literal is not None else 0)
        if text_count > MAX_TEXT:
            raise ValueError('展开后的字段文本超过 50 万字符，未截断导入')
        output.append({'pointer': pointer, 'kind': kind, 'value': literal, 'children': len(item) if container else 0})
        entries = item.items() if isinstance(item, dict) else enumerate(item) if isinstance(item, list) else []
        for key, child in entries:
            if isinstance(item, dict) and not isinstance(key, str):
                raise ValueError('YAML 字段名必须是字符串')
            escaped = str(key).replace('~', '~0').replace('/', '~1')
            walk(child, pointer + '/' + escaped, depth + 1)
        if container:
            active.remove(id(item))

    walk(value, '', 0)
    return output


if __name__ == '__main__':
    try:
        text = sys.stdin.buffer.read(50 * 1024 * 1024 + 1).decode('utf-8-sig')
        if len(text) > MAX_TEXT:
            raise ValueError('结构化文件文本超过 50 万字符，未截断导入')
        if not text.strip():
            raise ValueError('结构化文件为空')
        if sys.argv[1] == 'json':
            value = json.loads(text, object_pairs_hook=unique_object, parse_int=NumberLiteral, parse_float=NumberLiteral)
        else:
            value = yaml_value(text)
        print(json.dumps(fields(value), ensure_ascii=False, allow_nan=False))
    except json.JSONDecodeError as exc:
        print(f'JSON 语法有误（第 {exc.lineno} 行，第 {exc.colno} 列）', file=sys.stderr)
        sys.exit(1)
    except ImportError:
        print('缺少 YAML 解析依赖，请运行项目的 scripts/setup-interview.sh 安装依赖', file=sys.stderr)
        sys.exit(1)
    except (ValueError, UnicodeError, RecursionError) as exc:
        print(str(exc) if not isinstance(exc, RecursionError) else '结构化资料嵌套过深，未导入', file=sys.stderr)
        sys.exit(1)
