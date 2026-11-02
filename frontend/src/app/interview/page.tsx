'use client';

import { useCallback, useEffect, useRef, useState } from 'react';
import Link from 'next/link';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { RemoteUtteranceQueue, RemoteUtterance, TranscriptInput, UtteranceEnd } from '@/lib/interview-scheduler';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { indexedDBService } from '@/services/indexedDBService';
import { MicrophoneState, MicrophoneTranscriptGate, stopInterviewAudio } from '@/lib/interview-audio';
import { Play, Pause, Square, Search, Send, Snowflake, FolderOpen, Settings2, FileText, Trash2 } from 'lucide-react';

type Source = { source_id: string; path: string; location: string; text: string; kind: string; allow_external: boolean };
type File = { path: string; bytes: number; eligible: boolean; reason: string };
type Choice = { path: string; kind: string; allow_external: boolean };
type Status = { key_configured: boolean; root: string; suggested_root: string; index_version: string; architecture: string; memory_bytes: number; system_version: string; aliases: string; facts: string; whisper_model: string | null; session_active: boolean };
type Turn = { role: 'remote' | 'self' | 'manual'; sequence_id?: number; id: string; question: string; confirmed_answer: string | null; suggestion: string; status: 'listening' | 'queued' | 'generating' | 'done' | 'error' | 'cancelled' | 'recorded'; sources: Source[]; time: string; timing: string };
type Stream = { question_id: string; revision: number; event: { type: string; text?: string; message?: string; evidence?: Source[]; retrieval_ms?: number; first_text_ms?: number; elapsed_ms?: number; usage?: { total_tokens?: number } | null } };
const kindNames: Record<string, string> = { personal: '个人事实汇总', evidence: '原始证据', practice: '练习参考', job: '岗位信息', technical: '技术笔记' };

export default function InterviewPage() {
  const recording = useRecordingState();
  const { resetAfterHistoryClear } = useTranscripts();
  const [status, setStatus] = useState<Status | null>(null);
  const [question, setQuestion] = useState('');
  const [answer, setAnswer] = useState('');
  const [sources, setSources] = useState<Source[]>([]);
  const [phase, setPhase] = useState('就绪');
  const [notice, setNotice] = useState('');
  const [settings, setSettings] = useState(false);
  const [busy, setBusy] = useState(false);
  const [generating, setGenerating] = useState(false);
  const [frozen, setFrozen] = useState(false);
  const [automatic, setAutomatic] = useState(false);
  const [queued, setQueued] = useState(0);
  const [audioAllowed, setAudioAllowed] = useState(false);
  const [cloudAllowed, setCloudAllowed] = useState(false);
  const [transcripts, setTranscripts] = useState<TranscriptInput[]>([]);
  const [root, setRoot] = useState('');
  const [files, setFiles] = useState<File[]>([]);
  const [filter, setFilter] = useState('');
  const [selected, setSelected] = useState<Record<string, Choice>>({});
  const [facts, setFacts] = useState('');
  const [aliases, setAliases] = useState('');
  const [key, setKey] = useState('');
  const [history, setHistory] = useState<Turn[]>([]);
  const historyRef = useRef<Turn[]>([]);
  historyRef.current = history;
  const clearingHistory = useRef(false);
  const acceptTranscripts = useRef(true);
  const [calls, setCalls] = useState(0);
  const [limit, setLimit] = useState(1000);
  const [following, setFollowing] = useState(true);
  const [editingSelfId, setEditingSelfId] = useState<string | null>(null);
  const spoken = useRef<TranscriptInput[]>([]);
  const feed = useRef<HTMLDivElement>(null);
  const followLatest = useRef(true);
  const [timing, setTiming] = useState('');
  const [usage, setUsage] = useState<string>('未知');
  const [report, setReport] = useState('');
  const [level, setLevel] = useState(0);
  const [micLevel, setMicLevel] = useState(0);
  const [recordMicrophone, setRecordMicrophone] = useState(true);
  const microphoneGate = useRef(new MicrophoneTranscriptGate());
  const microphoneChanging = useRef(false);
  const microphoneVersion = useRef(0);
  const applyMicrophoneState = useCallback((state: MicrophoneState) => {
    microphoneGate.current.apply(state);
    if (state.recording) setRecordMicrophone(state.enabled);
    if (!state.enabled) setMicLevel(0);
  }, []);
  const [confirmedAnswer, setConfirmedAnswer] = useState('');
  const active = useRef<string | null>(null);
  const current = useRef({ question: '', answer: '' });
  const utterances = useRef(new RemoteUtteranceQueue());
  const inFlight = useRef(false);
  const live = useRef({ automatic, frozen, cloudAllowed, calls, limit });
  live.current = { automatic, frozen, cloudAllowed, calls, limit };
  const submitRef = useRef<(turn: RemoteUtterance) => void>(() => {});
  const error = useCallback((e: unknown) => setNotice(String(e)), []);

  const refresh = useCallback(async () => {
    const s = await invoke<Status>('interview_status');
    setStatus(s); setRoot(s.root || s.suggested_root || ""); setFacts(s.facts); setAliases(s.aliases);
  }, []);
  const cancel = useCallback(async () => {
    const cancelledId = active.current;
    const pendingIds = new Set(utterances.current.list().map(t => t.id));
    setHistory(h => h.map(t => t.id === cancelledId && t.status === 'generating' ? { ...t, status: 'cancelled' as const } : pendingIds.has(t.id) ? { ...t, status: 'recorded' as const } : t));
    active.current = null; inFlight.current = false; utterances.current.clear(); setQueued(0); setGenerating(false); setPhase('已取消 · 保留部分答案');
    await invoke('answer_cancel');
  }, []);
  const submit = useCallback(async (q: string, automated = false, turnId?: string) => {
    if (!cloudAllowed) { setNotice('请先勾选允许将对方发言、双方近期对话与选定证据发送给 DeepSeek。'); return; }
    if (!q.trim()) return;
    if (editingSelfId && !automated) {
      const turn = history.find(t => t.id === editingSelfId);
      spoken.current = spoken.current.map(t => t.sequence_id === turn?.sequence_id ? { ...t, text: q } : t);
      setHistory(h => h.map(t => t.id === editingSelfId ? { ...t, question: q } : t));
      setEditingSelfId(null); setQuestion(''); return;
    }
    if (calls >= limit) { setNotice('已达到本次会话请求软上限，可在下方手动增加 20 次。'); return; }
    inFlight.current = true; setFrozen(false);
    const previousId = active.current;
    const id = turnId || crypto.randomUUID(); active.current = id;
    current.current = { question: q, answer: '' };
    setHistory(h => {
      const next: Turn = { role: automated ? 'remote' : 'manual', id, question: q, suggestion: '', confirmed_answer: null, status: 'generating', sources: [], time: new Date().toLocaleTimeString('zh-CN'), timing: '' };
      const updated = h.map(t => t.id === previousId && t.status === 'generating' ? { ...t, status: 'cancelled' as const } : t);
      return updated.some(t => t.id === id) ? updated.map(t => t.id === id ? { ...next, time: t.time } : t) : [...updated, next];
    });
    if (!automated) setQuestion('');
    setAnswer(''); setSources([]); setNotice(''); setTiming(''); setUsage('未知'); setGenerating(true); setPhase('检索与生成中');
    try {
      await invoke('answer_start', { questionId: id, revision: 0, question: q, history: history.filter(t => t.status === 'done' && t.role !== 'self').slice(-3).map(t => ({ question: t.question, confirmed_answer: t.confirmed_answer })), dialogue: spoken.current.slice(-12).map(t => ({ source: t.source, text: t.text })) });
      if (active.current === id) { setCalls(n => n + 1); setStatus(s => s ? { ...s, session_active: true } : s); }
    } catch (e) { if (active.current === id) { inFlight.current = false; setAutomatic(false); setHistory(h => h.map(t => t.id === id ? { ...t, status: 'error' } : t)); error(e); setGenerating(false); setPhase('请求失败 · 自动队列已暂停，可重试当前句'); } }
  }, [cloudAllowed, calls, limit, history, editingSelfId, error]);
  submitRef.current = turn => void submit(turn.text, true, turn.id);

  useEffect(() => {
    if (!isTauri()) { setNotice('此页面需在 Meetily 桌面应用运行。浏览器预览不能调用本地资料、音频或 API。'); return; }
    void refresh().catch(error);
    let disposed = false; const unlisten: UnlistenFn[] = [];
    const add = async <T,>(name: string, fn: (payload: T) => void) => {
      const stop = await listen<T>(name, e => fn(e.payload));
      if (disposed) stop(); else unlisten.push(stop);
    };
    void add<Stream>('interview-answer', ({ question_id, revision, event }) => {
      if (question_id !== active.current || revision !== 0) return;
      if (event.type === 'sources') { setSources(event.evidence || []); setTiming(`检索 ${event.retrieval_ms} ms`); setHistory(h => h.map(t => t.id === question_id ? { ...t, sources: event.evidence || [], timing: `检索 ${event.retrieval_ms} ms` } : t)); }
      if (event.type === 'delta') { current.current.answer += event.text || ''; setAnswer(current.current.answer); const text = current.current.answer; setHistory(h => h.map(t => t.id === question_id ? { ...t, suggestion: text } : t)); setPhase('正在生成'); }
      if (event.type === 'warning') setNotice(event.message || '');
      if (event.type === 'error') { setHistory(h => h.map(t => t.id === question_id ? { ...t, status: 'error' } : t)); inFlight.current = false; setAutomatic(false); setNotice(event.message || '请求失败'); setPhase('未完成 · 自动队列已暂停，可重试当前句'); setGenerating(false); }
      if (event.type === 'done') {
        inFlight.current = false; setGenerating(false); setPhase('建议已就绪'); setTiming(t => `${t} · 首字 ${event.first_text_ms ?? '?'} ms · 总计 ${event.elapsed_ms} ms`);
        setUsage(event.usage?.total_tokens === undefined ? '未知' : `${event.usage.total_tokens} tokens`);
        setHistory(h => h.map(t => t.id === question_id ? { ...t, status: 'done', timing: `${t.timing} · 首字 ${event.first_text_ms ?? '?'} ms · 总计 ${event.elapsed_ms} ms` } : t));
      }
    }).catch(error);
    void add<TranscriptInput>('transcript-update', event => {
      if (clearingHistory.current || !acceptTranscripts.current) return;
      if (event.is_partial) return;
      if (!microphoneGate.current.accepts(event)) return;
      setTranscripts(previous => [...previous.filter(t => t.sequence_id !== event.sequence_id), event].slice(-60));
      spoken.current = [...spoken.current.filter(t => t.sequence_id !== event.sequence_id), event].slice(-120);
      if (event.source === 'Microphone') {
        setHistory(h => [...h.filter(t => t.id !== `mic-${event.sequence_id}`), { role: 'self', sequence_id: event.sequence_id, id: `mic-${event.sequence_id}`, question: event.text, suggestion: '', confirmed_answer: null, status: 'done', sources: [], time: event.timestamp, timing: '' }]);
        return;
      }
      const draft = utterances.current.receive(event);
      if (draft) setHistory(h => {
        const row: Turn = { role: 'remote', id: draft.id, question: draft.text, suggestion: '', confirmed_answer: null, status: 'listening', sources: [], time: draft.time, timing: '' };
        return h.some(t => t.id === row.id) ? h.map(t => t.id === row.id ? row : t) : [...h, row];
      });
    }).catch(error);
    void add<UtteranceEnd>('interview-utterance-end', event => {
      if (clearingHistory.current || !acceptTranscripts.current) return;
      const l = live.current;
      const result = utterances.current.finish(event, l.automatic && !l.frozen && l.cloudAllowed);
      if (result.turn) setHistory(h => h.map(t => t.id === result.turn!.id ? { ...t, status: result.status === 'queued' ? 'queued' : 'recorded' } : t));
      if (result.status === 'full' || result.status === 'failed') {
        live.current.automatic = false; setAutomatic(false);
        setNotice(result.status === 'full' ? '待处理超过 20 段或整段发言超过 1500 字，已暂停自动发送；原文保留，可手动分段处理。' : '这段对方发言存在转写失败，已保留可用文字并暂停自动回答，请核对后重试。');
      }
      setQueued(utterances.current.size);
    }).catch(error);
    void add<{ userMessage?: string }>('transcription-error', event => { setAutomatic(false); setNotice(event.userMessage || '转写失败，已暂停自动发送'); }).catch(error);
    void add<string>('recording-error', message => { setAutomatic(false); setNotice(message || '采集失败，已暂停自动发送'); }).catch(error);
    void add<MicrophoneState>('interview-microphone-state', state => { if (!microphoneChanging.current) applyMicrophoneState(state); }).catch(error);
    const levelTimer = setInterval(() => {
      if (microphoneChanging.current) return;
      const version = microphoneVersion.current;
      void invoke<{system:number;microphone:number;microphone_state:MicrophoneState}>('interview_audio_level').then(levels => {
        if (microphoneChanging.current || version !== microphoneVersion.current) return;
        setLevel(levels.system); setMicLevel(levels.microphone_state.enabled ? levels.microphone : 0); applyMicrophoneState(levels.microphone_state);
      }).catch(() => {});
    }, 500);
    const timer = setInterval(() => {
      const l = live.current;
      if (!l.automatic || l.frozen || !l.cloudAllowed || inFlight.current) return;
      if (l.calls >= l.limit) {
        if (utterances.current.size) { setAutomatic(false); setNotice('已达到会话请求上限；增加额度后可重新勾选自动发送，继续待处理发言。'); }
        return;
      }
      const turn = utterances.current.take();
      if (turn) { setQueued(utterances.current.size); submitRef.current(turn); }
    }, 100);
    return () => { disposed = true; unlisten.forEach(stop => stop()); clearInterval(timer); clearInterval(levelTimer); active.current = null; void invoke('answer_cancel').catch(() => {}); };
  }, [refresh, error, applyMicrophoneState]);

  useEffect(() => {
    if (!recording.isActive) setAutomatic(false);
  }, [recording.isActive]);
  useEffect(() => { if (followLatest.current && feed.current) feed.current.scrollTop = feed.current.scrollHeight; }, [history]);
  const run = async (work: () => Promise<unknown>) => { setBusy(true); setNotice(''); try { await work(); } catch (e) { error(e); } finally { setBusy(false); } };
  const localSearch = () => run(async () => { setAutomatic(false); await cancel(); setSources(await invoke<Source[]>('rag_search', { question })); setPhase('仅本地检索 · 未请求模型'); });
  const end = () => run(async () => {
    setAutomatic(false); live.current.automatic = false; await cancel();
    await stopInterviewAudio();
    microphoneGate.current.close(); setMicLevel(0); setLevel(0);
    await invoke('session_clear');
    setQuestion(''); setConfirmedAnswer(''); utterances.current.reset(); setQueued(0);
    current.current = { question: '', answer: '' }; setPhase('会话已结束 · 历史已保留，可导出'); await refresh();
  });
  const exportSession = () => {
    const data = JSON.stringify({ exported_at: new Date().toISOString(), mode: 'AI 建议不等于本人实际回答', spoken_dialogue: spoken.current, transcripts, history, current: current.current, calls, usage }, null, 2);
    const url = URL.createObjectURL(new Blob([data], { type: 'application/json' }));
    const a = document.createElement('a'); a.href = url; a.download = 'interview-session.json'; a.click(); URL.revokeObjectURL(url);
  };
  const clearHistory = () => run(async () => {
    if (clearingHistory.current) return;
    clearingHistory.current = true; acceptTranscripts.current = false;
    live.current.automatic = false; setAutomatic(false);
    indexedDBService.suspendRecordingWrites();
    setPhase('正在停止采集并将历史移入废纸篓');
    let archivePath = '';
    try {
      await cancel();
      const capture = await invoke<{is_recording: boolean}>('get_recording_state');
      if (capture.is_recording) await stopInterviewAudio();
      microphoneGate.current.close();
      const recovery = await indexedDBService.snapshotInterviewHistory();
      const report = await invoke<{archive_path:string; recording_folders:number; database_meetings:number}>('interview_history_clear', {
        snapshot: { current_session: { history: historyRef.current, transcripts, spoken_dialogue: spoken.current, current: current.current, calls, usage }, recovery }
      });
      archivePath = report.archive_path;
      await indexedDBService.removeArchivedInterviewHistory(recovery);
      resetAfterHistoryClear();
      utterances.current.reset(); spoken.current = []; historyRef.current = [];
      active.current = null; inFlight.current = false; current.current = { question:'', answer:'' };
      setHistory([]); setTranscripts([]); setQuestion(''); setAnswer(''); setSources([]);
      setConfirmedAnswer(''); setEditingSelfId(null); setQueued(0); setCalls(0); setLimit(1000);
      setTiming(''); setUsage('未知'); setGenerating(false); setFrozen(false); setLevel(0); setMicLevel(0);
      followLatest.current = true; setFollowing(true);
      setPhase('历史已清空 · 可重新开始');
      setNotice(`历史已移入废纸篓：${report.recording_folders} 个录音记录目录、${report.database_meetings} 条已保存会议，以及本次对话和恢复缓存。个人资料库已保留。点击“开始持续聆听与回答”进行新测试。`);
      await refresh();
    } catch (e) {
      setPhase('清理未完成 · 已停止自动回答');
      throw archivePath ? new Error(`${String(e)}；完整备份已保留在废纸篓：${archivePath}`) : e;
    } finally { clearingHistory.current = false; }
  });
  const toggleMicrophone = (enabled: boolean) => run(async () => {
    microphoneVersion.current++;
    microphoneChanging.current = true;
    microphoneGate.current.close();
    try {
      if (recording.isRecording) {
        applyMicrophoneState(await invoke<MicrophoneState>('interview_microphone_set', { enabled }));
      } else { setRecordMicrophone(enabled); }
      setMicLevel(0);
    } catch (e) {
      const levels = await invoke<{microphone_state:MicrophoneState}>('interview_audio_level').catch(() => null);
      if (levels) applyMicrophoneState(levels.microphone_state);
      throw e;
    } finally { microphoneChanging.current = false; }
  });
  const toggleFile = (file: File, on: boolean) => setSelected(s => {
    const next = { ...s }; if (on) next[file.path] = { path: file.path, kind: /面试资料|面试准备|练习/.test(file.path) ? 'practice' : 'personal', allow_external: true }; else delete next[file.path]; return next;
  });

  return <main className="mx-auto max-w-[1500px] p-6 space-y-5 text-slate-800">
    <header className="flex flex-wrap items-start justify-between gap-3 border-b pb-4">
      <div><p className="text-xs tracking-widest text-slate-500">MEETILY COMMUNITY · 本地修改版</p><h1 className="text-2xl font-semibold mt-1">面试辅助</h1><p className="text-sm text-slate-500 mt-1">技术面试练习 · 先核对事实，再组织表达</p></div>
      <div className="flex items-center gap-2"><span className="rounded-full bg-slate-100 px-3 py-1 text-xs">{phase}</span><Button variant="outline" onClick={() => setSettings(!settings)}><Settings2 className="mr-2 h-4 w-4" />资料与设置</Button></div>
    </header>
    {notice && <div role="alert" className="rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm break-words">{notice}</div>}
    <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs text-slate-500">
      <span>DeepSeek Flash · thinking 关闭 · {status?.key_configured ? 'Key 已配置' : 'Key 未配置'}</span>
      <span>索引 v{status?.index_version || '0'}{status?.session_active ? ' · 会话锁定' : ''}</span>
      <span>{timing || '等待本次请求耗时'}</span><span>本次调用 {calls} / {limit} · 最近用量 {usage}</span>
    </div>
    {settings && <section className="rounded-xl border bg-white p-5 space-y-4">
      <h2 className="font-semibold">只读资料索引</h2>
      <p className="text-xs text-slate-500">{status ? `${status.architecture} · macOS ${status.system_version} · ${(status.memory_bytes / 1024 ** 3).toFixed(0)} GB · Whisper：${status.whisper_model || '未加载'}` : '等待桌面自检'}</p>
      <div className="flex gap-2"><Input aria-label="资料根目录" placeholder="选择已有个人资料目录" value={root} onChange={e => { setRoot(e.target.value); setFiles([]); setSelected({}); }} /><Button variant="outline" onClick={() => run(async () => { const p = await invoke<string | null>('profile_choose'); if (p) { setRoot(p); setFiles([]); setSelected({}); } })}><FolderOpen className="h-4 w-4" /></Button><Button disabled={busy || !root || status?.session_active} onClick={() => run(async () => { setFiles(await invoke<File[]>('profile_scan', { root })); setSelected({}); })}>扫描预览</Button></div>
      <Button variant="outline" disabled={busy || !root || status?.session_active} onClick={() => run(async () => {
        const paths = ['background.md', 'resume.md', 'reference-answers.md'];
        const candidates = (await invoke<File[]>('profile_scan', { root })).filter(f => paths.includes(f.path));
        setFiles(candidates); setSelected(Object.fromEntries(candidates.filter(f => f.eligible).map(f => [f.path, { path: f.path, kind: f.path === 'reference-answers.md' ? 'practice' : 'personal', allow_external: true }]))); setFilter('');
        if (!candidates.length) setNotice('当前目录中未找到推荐资料，请确认根目录或手动扫描。');
      })}>预览推荐三份资料（背景 + 简历 + 参考回答）</Button>
      {files.length > 0 && <><Input aria-label="筛选文件" value={filter} onChange={e => setFilter(e.target.value)} placeholder="筛选文件名，例如：背景、resume" /><div className="max-h-64 overflow-auto divide-y rounded border">
        {files.filter(f => f.path.toLowerCase().includes(filter.toLowerCase())).slice(0,300).map(file => <div key={file.path} className="p-2 text-xs flex flex-wrap items-center gap-2">
          <input aria-label={`纳入 ${file.path}`} type="checkbox" disabled={!file.eligible || status?.session_active} checked={!!selected[file.path]} onChange={e => toggleFile(file, e.target.checked)} /><span className="flex-1 min-w-40 break-all">{file.path}</span><span className="text-slate-400">{(file.bytes/1024).toFixed(0)} KB · {file.reason}</span>
          {selected[file.path] && <><select aria-label="资料类型" className="border rounded p-1" value={selected[file.path].kind} onChange={e => setSelected(s => ({ ...s, [file.path]: { ...s[file.path], kind: e.target.value } }))}>{Object.entries(kindNames).map(([v,n]) => <option key={v} value={v}>{n}</option>)}</select><label><input type="checkbox" checked={selected[file.path].allow_external} onChange={e => setSelected(s => ({ ...s, [file.path]: { ...s[file.path], allow_external: e.target.checked } }))} /> 允许相关片段外发</label></>}
        </div>)}
      </div><p className="text-xs text-slate-500">扫描 {files.length} 项，当前选择 {Object.keys(selected).length} 项。列表最多展示 300 项，可输入名称筛选。重新导入会移除未选择项目的活动索引。</p><Button disabled={busy || status?.session_active} onClick={() => run(async () => { const r = await invoke('profile_import', { root, selected: Object.values(selected) }); setReport(JSON.stringify(r, null, 2)); await refresh(); })}>导入 / 更新所选资料</Button></>}
      {report && <pre className="max-h-48 overflow-auto bg-slate-50 p-3 rounded text-xs whitespace-pre-wrap">{report}</pre>}
      <div className="grid md:grid-cols-2 gap-4"><label className="text-sm">本人确认的事实卡（可留空）<textarea className="w-full border rounded p-2 mt-1 h-28 text-sm" maxLength={2200} value={facts} onChange={e => setFacts(e.target.value)} placeholder="只填写本人核对过的事实与来源，不粘贴 AI 建议。" /></label><label className="text-sm">术语别名（每行一组，英文逗号分隔）<textarea className="w-full border rounded p-2 mt-1 h-28 text-sm" value={aliases} onChange={e => setAliases(e.target.value)} /></label></div>
      <div className="flex gap-2"><Input type="password" aria-label="临时 API Key" autoComplete="off" value={key} onChange={e => setKey(e.target.value)} placeholder="替换本进程 API Key（不回显、不写入前端存储）" /><Button disabled={busy || status?.session_active} onClick={() => run(async () => { await invoke('interview_config', { facts, aliases, key: key || null }); setKey(''); await refresh(); })}>保存配置</Button><Link href="/settings" className="text-sm underline self-center whitespace-nowrap">原生音频设置</Link></div>
    </section>}
    <section className="rounded-xl bg-slate-50 border p-4 space-y-3">
      <div className="flex flex-wrap gap-5 text-sm"><label><input type="checkbox" checked={audioAllowed} onChange={e => setAudioAllowed(e.target.checked)} /> 已获允许使用转写</label><label><input type="checkbox" checked={cloudAllowed} onChange={e => { setCloudAllowed(e.target.checked); if (!e.target.checked) { setAutomatic(false); void cancel().catch(error); } }} /> 允许向 DeepSeek 发送对方发言、双方近期对话与选定证据</label></div>
      <div className="flex flex-wrap gap-2 items-center">
        <Button disabled={!audioAllowed || !cloudAllowed || recording.isRecording || busy} onClick={() => run(async () => {
          utterances.current.reset(); setQueued(0); setFrozen(false); setAutomatic(true);
          acceptTranscripts.current = true;
          live.current.automatic = true;
          microphoneChanging.current = true;
          microphoneVersion.current++;
          try {
            applyMicrophoneState(await invoke<MicrophoneState>('interview_audio_start', { recordMicrophone }));
            setPhase('监听中 · 对方说完后回答');
          } catch (e) { setAutomatic(false); live.current.automatic = false; throw e; }
          finally { microphoneChanging.current = false; }
        })}><Play className="h-4 w-4 mr-1" />开始持续聆听与回答</Button>
        <Button variant="outline" disabled={!recording.isRecording || busy} onClick={() => run(async () => {
          if (recording.isPaused) { await invoke('resume_recording'); setAutomatic(cloudAllowed); }
          else { setAutomatic(false); await cancel(); await invoke('pause_recording'); }
        })}><Pause className="h-4 w-4 mr-1" />{recording.isPaused ? '恢复' : '暂停'}</Button>
        <Button variant="outline" disabled={!recording.isRecording || busy} onClick={() => run(async () => {
          live.current.automatic = false; setAutomatic(false); await cancel(); await stopInterviewAudio();
          microphoneGate.current.close(); setMicLevel(0); setLevel(0); setPhase('采集已停止 · 可直接重新开始');
        })}><Square className="h-4 w-4 mr-1" />停止采集</Button>
        <Button variant="outline" disabled={busy} onClick={clearHistory}><Trash2 className="h-4 w-4 mr-1" />清空历史，重新开始</Button>
        <span className="text-xs text-slate-500">{recording.isRecording ? recording.isPaused ? '采集已暂停' : recordMicrophone ? '正在采集系统输出和麦克风' : '仅采集对方系统音频 · 我的麦克风已关闭' : '采集未开始'} · 系统 {Math.round(level * 100)} · 麦克风 {Math.round(micLevel * 100)} · 按采集通道区分</span>
      </div>
      <div className="text-sm space-y-1"><label><input type="checkbox" checked={recordMicrophone} disabled={busy || recording.isStopping} onChange={e => void toggleMicrophone(e.target.checked)} /> 记录我的声音（本软件麦克风）</label><p className="text-xs text-slate-500">与飞书静音独立。可在采集中切换；关闭后释放本软件麦克风，丢弃未完成的麦克风转写，已显示的记录保留。单人扮演对方朗读时请关闭，回答时再打开。</p></div>
      <p className="text-xs text-slate-500">关闭无关系统声音。当前不保存原始录音；转写会留存，可通过“清空历史，重新开始”移入废纸篓。</p>
      <div className="flex flex-wrap gap-4 text-xs"><label><input type="checkbox" disabled={!cloudAllowed || !recording.isActive} checked={automatic} onChange={e => setAutomatic(e.target.checked)} /> 对方说完后自动回答；我的发言只作为上下文</label><span>待处理 {queued} 段 · 依次回答 · 完整记录在下方会话中</span></div>
      <p className="text-xs text-slate-500">对方说话期间持续转写，停顿约 1.5 秒后合并整段生成一次回答；我的发言只转写并供后续追问参考，不单独请求 AI。请使用耳机，减少串音。</p>
    </section>
    <section className="rounded-xl border bg-white overflow-hidden">
      <div className="flex items-center justify-between border-b px-4 py-3 gap-3"><div><h2 className="font-semibold">面试对话</h2><p className="text-xs text-slate-500">持续转写到结束 · {history.length} 条记录 · 待处理 {queued} 段</p></div><div className="flex gap-2"><Button size="sm" variant={frozen ? 'default' : 'outline'} onClick={() => { const next = !frozen; setFrozen(next); if (next) { void cancel().catch(error); setPhase('我在回答 · 已冻结'); } }}>{frozen ? '解除冻结' : '我在回答 / 冻结'}</Button><Button size="sm" variant="outline" disabled={!generating && !queued} onClick={() => cancel().catch(error)}>取消当前与队列</Button></div></div>
      <div ref={feed} onScroll={() => { const el = feed.current; if (el) { const near = el.scrollHeight - el.scrollTop - el.clientHeight < 90; followLatest.current = near; setFollowing(near); } }} className="h-[58vh] min-h-80 overflow-y-auto p-5 space-y-6 bg-slate-50/60" aria-live="off">
        {!history.length && <div className="py-16 text-center text-slate-400"><p className="text-lg">在面试开始前开启，之后持续聆听</p><p className="text-sm mt-2">双方发言持续保留，只在对方说完后生成回答。</p></div>}
        {history.map(turn => <article key={turn.id} className="space-y-3">
          <div className="ml-auto max-w-[90%] rounded-xl bg-slate-800 text-white px-4 py-3"><p className="text-xs text-slate-300 mb-1">{turn.role === 'self' ? '我的回答 · 麦克风转写' : turn.role === 'remote' ? '对端所在的系统音频' : '手动输入'} · {turn.time}</p><p className="whitespace-pre-wrap break-words">{turn.question}</p><button className="text-xs text-slate-300 underline mt-2" onClick={() => { setQuestion(turn.question); setEditingSelfId(turn.role === 'self' ? turn.id : null); }}>{turn.role === 'self' ? '纠正我的回答' : '编辑后重问'}</button></div>
          {turn.role !== 'self' && !['recorded', 'listening', 'queued'].includes(turn.status) && <div className="mr-auto max-w-[95%] rounded-xl border bg-white px-5 py-4"><p className="text-xs text-slate-500 mb-2">AI · 结合背景理解与回答 {turn.status === 'generating' ? '· 正在生成' : turn.status === 'error' ? '· 请求失败' : turn.status === 'cancelled' ? '· 已取消' : ''}</p><div className="whitespace-pre-wrap break-words text-base leading-7">{turn.suggestion || (turn.status === 'generating' ? '正在检索资料并理解整段发言…' : '未生成完整回答，可编辑后重试。')}</div>
            {turn.sources.length > 0 && <details className="border-t mt-4 pt-2"><summary className="text-xs text-slate-500 cursor-pointer">参考来源 · {turn.sources.length} 个片段</summary><div className="space-y-2 mt-2">{turn.sources.map(source => <details key={source.source_id} className="rounded bg-slate-50 p-2"><summary className="text-xs break-all cursor-pointer">{kindNames[source.kind]} · {source.path} · {source.location}</summary><p className="text-xs text-slate-500 mt-1">{source.source_id}</p><p className="whitespace-pre-wrap text-sm mt-2">{source.text}</p></details>)}</div></details>}
            {turn.timing && <p className="text-[11px] text-slate-400 mt-3">{turn.timing}</p>}
          </div>}
          {turn.status === 'recorded' && <p className="text-xs text-right text-slate-500">已保留转写 · 自动生成已暂停，可编辑后重问</p>}
          {turn.status === 'listening' && <p className="text-xs text-right text-slate-500">对方仍在说话 · 只更新转写，等待发言结束</p>}
          {turn.status === 'queued' && <p className="text-xs text-right text-slate-500">整段发言已结束 · 排队等待回答</p>}
          {turn.role === 'self' && <p className="text-xs text-right text-slate-500">只加入后续上下文 · 不触发 AI 回答 · 转写可纠正</p>}
        </article>)}
      </div>
      {!following && <div className="flex justify-center border-t p-2"><Button variant="outline" size="sm" onClick={() => { followLatest.current = true; setFollowing(true); if (feed.current) feed.current.scrollTop = feed.current.scrollHeight; }}>回到最新对话</Button></div>}
      <div className="border-t p-4 space-y-2"><label htmlFor="question" className="text-xs text-slate-500">{editingSelfId ? '纠正本人回答 · 保存后用于后续上下文' : '手动纠正或补充问题 · ⌘ Enter 发送'}</label><div className="flex gap-2 items-end"><textarea id="question" value={question} maxLength={1500} onChange={e => setQuestion(e.target.value)} onKeyDown={e => { if ((e.metaKey || e.ctrlKey) && e.key === 'Enter') { e.preventDefault(); void submit(question); } }} className="flex-1 h-20 border rounded-lg p-3 text-sm resize-y" placeholder="可选：编辑识别内容，或手动补充一句话" /><Button disabled={!question.trim() || !cloudAllowed || busy} onClick={() => submit(question)}><Send className="h-4 w-4 mr-1" />{editingSelfId ? '保存修正' : '发送'}</Button><Button variant="outline" disabled={!question.trim() || busy} onClick={localSearch}>本地检索</Button></div>
        {phase.startsWith('仅本地') && <div className="text-xs space-y-2">{sources.map(source => <details key={source.source_id}><summary>{source.path} · {source.location}</summary><p className="whitespace-pre-wrap">{source.text}</p></details>)}</div>}
      </div>
    </section>
    <details className="border rounded-lg p-4 text-sm"><summary>原始转写与本人回答记录</summary><div className="max-h-48 overflow-auto space-y-2 my-3">{transcripts.map(t => <button key={t.sequence_id} className="block text-left w-full p-2 rounded bg-slate-50" onClick={() => setQuestion(t.text)}>{t.timestamp} · {t.text}</button>)}</div><textarea aria-label="本轮实际回答" className="border rounded w-full h-20 p-2" value={confirmedAnswer} onChange={e => setConfirmedAnswer(e.target.value)} placeholder="可选：记录最近一轮自己实际说过的回答，供后续追问使用。" /><Button variant="outline" disabled={!confirmedAnswer.trim() || !history.length} onClick={() => { setHistory(h => { const last = h.map(t => t.role !== 'self' && t.status === 'done').lastIndexOf(true); return h.map((t,i) => i === last ? { ...t, confirmed_answer: confirmedAnswer } : t); }); setConfirmedAnswer(''); }}>确认本人实际回答</Button></details>
    <footer className="flex flex-wrap justify-between gap-3 border-t pt-4 text-xs text-slate-500"><span>转写、建议与源资料各自独立。实际飞书链路与 60 分钟稳定性尚待验收。</span><div className="flex gap-2">{calls >= limit && <Button variant="outline" size="sm" onClick={() => setLimit(l => l + 20)}>继续 20 次</Button>}<Button variant="outline" size="sm" onClick={exportSession}>导出本次记录</Button><Button variant="outline" size="sm" disabled={busy} onClick={end}>结束会话并保留历史</Button></div></footer>
  </main>;
}
