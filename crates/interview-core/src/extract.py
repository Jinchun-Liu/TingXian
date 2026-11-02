"""One-shot read-only DOCX/PDF extraction. JSONL output, never a server."""
import json
import sys
from pathlib import Path

MAX_TEXT = 500_000

def extract(path):
    if path.suffix.lower() == '.docx':
        from docx import Document
        from docx.text.paragraph import Paragraph
        from docx.table import Table
        doc = Document(path)
        paragraph, table = 0, 0
        for block in doc.iter_inner_content():
            if isinstance(block, Paragraph):
                paragraph += 1
                if block.text.strip():
                    yield {'location': f'段落 {paragraph}', 'text': block.text}
            elif isinstance(block, Table):
                table += 1
                rows = [[cell.text for cell in row.cells] for row in block.rows]
                if rows:
                    header = ' | '.join(rows[0])
                    for i, row in enumerate(rows[1:] or rows):
                        yield {'location': f'表 {table} 行 {i + 1}', 'text': header + '\n' + ' | '.join(row)}
    elif path.suffix.lower() == '.pdf':
        from pypdf import PdfReader
        reader = PdfReader(path)
        for number, page in enumerate(reader.pages, 1):
            text = page.extract_text() or ''
            if not text.strip():
                raise ValueError(f'第 {number} 页无可抽取文本，需人工处理；未启用 OCR')
            yield {'location': f'PDF 第 {number} 页', 'text': text}
    else:
        raise ValueError('不支持的格式')

if __name__ == '__main__':
    try:
        # Validate the whole document before writing so partial output is never indexed.
        segments = list(extract(Path(sys.argv[1])))
        if sum(len(s['text']) for s in segments) > MAX_TEXT:
            raise ValueError('抽取文本超过 50 万字符，未截断导入')
        if not segments:
            raise ValueError('文档没有可抽取文本')
        for segment in segments:
            print(json.dumps(segment, ensure_ascii=False))
    except Exception as exc:
        print(str(exc), file=sys.stderr)
        sys.exit(1)
