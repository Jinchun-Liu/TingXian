'use client';
import { useEffect, useRef, useState } from 'react';
import { invoke } from '@tauri-apps/api/core';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { includesField, StructuredPreview, StructuredSelection } from '@/lib/interview-structured';

const kinds: Record<string, string> = { object: '对象', array: '数组', string: '字符串', number: '数字', boolean: '布尔值', null: 'null 空值' };

export function StructuredFields({ root, path, value, locked, onChange, onBusyChange }: {
  root: string; path: string; value?: StructuredSelection | null; locked: boolean;
  onChange: (selection: StructuredSelection | null) => void; onBusyChange: (busy: boolean) => void;
}) {
  const [preview, setPreview] = useState<StructuredPreview | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState('');
  const [filter, setFilter] = useState('');
  const request = useRef(0);
  const reading = useRef(false);
  useEffect(() => () => { request.current++; if (reading.current) onBusyChange(false); }, [onBusyChange]);
  const load = async () => {
    if (locked || reading.current) return;
    const id = ++request.current;
    reading.current = true; setLoading(true); onBusyChange(true); setError(''); setPreview(null); onChange(null);
    try {
      const next = await invoke<StructuredPreview>('profile_structured_preview', { root, path });
      if (id !== request.current) return;
      if (next.path !== path) throw Error('资料文件已变化，请重新预览');
      setPreview(next);
    } catch (e) { if (id === request.current) setError(String(e)); }
    finally { if (id === request.current) { reading.current = false; setLoading(false); onBusyChange(false); } }
  };
  const pointers = preview && value?.file_hash === preview.file_hash && value.root === preview.root ? value.pointers : [];
  const change = (pointer: string, checked: boolean) => {
    if (!preview || locked || loading) return;
    const next = checked ? [...pointers.filter(p => !includesField(pointer, p)), pointer] : pointers.filter(p => p !== pointer);
    onChange(next.length ? { root: preview.root, file_hash: preview.file_hash, pointers: next.sort() } : null);
  };
  const fields = preview?.fields.filter(f => f.pointer !== '' && f.pointer.toLowerCase().includes(filter.toLowerCase())) || [];
  const rootField = preview?.fields.find(f => f.pointer === '');
  return <div className="w-full rounded border bg-slate-50 p-3 space-y-2" aria-label={`结构化字段 ${path}`}>
    <div className="flex flex-wrap items-center gap-3"><Button size="sm" variant="outline" disabled={locked || loading} onClick={load}>{loading ? '正在读取字段…' : preview ? '重新预览字段' : '预览字段'}</Button><span>先选择字段或整份文件，再导入。预览不会写入索引。</span></div>
    {error && <p role="alert" className="text-red-700">{error}</p>}
    {preview && <>
      <label className="flex gap-2 items-center"><input type="checkbox" aria-label={`整份文件 ${path}`} disabled={locked || loading} checked={pointers.includes('')} onChange={e => change('', e.target.checked)} />整份文件（包含全部字段）</label>
      {rootField && rootField.preview !== '' && <p className="whitespace-pre-wrap break-words">根值 · {kinds[rootField.kind] || rootField.kind}：{rootField.preview}{rootField.characters > 160 ? `…（共 ${rootField.characters} 字符）` : ''}</p>}
      <p className="text-slate-500">选择对象或数组会包含其下所有字段。值保留类型，null、0 和空字符串分别显示。</p>
      {/\.ya?ml$/i.test(path) && <p className="text-slate-500">YAML 使用 1.1 标量规则，需保留 yes/on 等原词时请在原文件中加引号。</p>}
      <Input aria-label={`筛选字段 ${path}`} value={filter} onChange={e => setFilter(e.target.value)} placeholder="筛选字段路径" />
      <div className="max-h-60 overflow-auto divide-y">
        {fields.slice(0,300).map(field => {
          const parentSelected = pointers.some(p => p !== field.pointer && includesField(p, field.pointer));
          return <label key={field.pointer} className="flex items-start gap-2 py-2">
            <input type="checkbox" aria-label={`字段 ${path} ${field.pointer}`} disabled={locked || loading || parentSelected} checked={pointers.some(p => includesField(p, field.pointer))} onChange={e => change(field.pointer, e.target.checked)} />
            <span className="min-w-0 break-words"><code>{field.pointer}</code> · {kinds[field.kind] || field.kind}{field.children > 0 ? ` · ${field.children} 个子项` : ''}{parentSelected ? ' · 随父字段纳入' : ''}{field.preview !== '' && <span className="block whitespace-pre-wrap text-slate-600">{field.preview}{field.characters > 160 ? `…（共 ${field.characters} 字符，预览截取前 160 字符）` : ''}</span>}</span>
          </label>;
        })}
      </div>
      <p>匹配 {fields.length} 个字段，最多显示前 300 个；可筛选定位。{pointers.length ? pointers.includes('') ? '已选择整份文件。' : `已选择 ${pointers.length} 个字段范围（含其子字段）。` : '尚未选择，不会导入。'}</p>
    </>}
  </div>;
}
