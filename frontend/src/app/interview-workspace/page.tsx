'use client';

import {LiveTranscription} from '@/components/interview/LiveTranscription';
import {useInterviewRecovery} from '@/lib/use-interview-recovery';
import {parseRecoverySnapshot,RecoverySnapshot} from '@/lib/interview-recovery';
import {RecoveryDrafts} from '@/components/interview/RecoveryDrafts';
import {SavedSessionPreview} from '@/components/interview/SavedSessionPreview';
import {useSessionCommands} from '@/lib/session-commands';
import {PreviousMeetings} from '@/components/interview/PreviousMeetings';
import {AudioSettings} from '@/components/interview/AudioSettings';
import {ChoiceMenu, ChoiceOption} from '@/components/ui/choice-menu';
import { useCallback, useEffect, useRef, useState } from 'react';
import Link from 'next/link';
import { invoke, isTauri } from '@tauri-apps/api/core';
import { listen, UnlistenFn } from '@tauri-apps/api/event';
import { Button } from '@/components/ui/button';
import { ResidueNotice } from '@/components/interview/ResidueNotice';
import { BackgroundLibrary } from '@/components/interview/BackgroundLibrary';
import { ProviderSettings } from '@/components/interview/ProviderSettings';
import { ConnectionCheck } from '@/components/interview/ConnectionCheck';
import {AnswerConversation} from '@/components/interview/AnswerConversation';
import {AnswerRequests,answerPresentation,presentationDefaults,readPresentationPreferences,PresentationPreferences} from '@/lib/interview-presentation';
import {QuestionComposer} from '@/components/interview/QuestionComposer';
import { interviewExit } from '@/lib/interview-exit';
import { DocumentReview } from '@/components/interview/DocumentReview';
import { FactReview } from '@/components/interview/FactReview';
import { ConflictNotice } from '@/components/interview/ConflictNotice';
import { StructuredFields } from '@/components/interview/StructuredFields';
import { isStructuredPath, StructuredSelection } from '@/lib/interview-structured';
import { SourceConflict } from '@/lib/interview-conflicts';
import { FactCard, legacyFactCard } from '@/lib/interview-facts';
import { DocumentMetadata, DocumentRecord, sourceStatusLabel } from '@/lib/interview-documents';
import { Source, Turn, SessionSnapshot, SessionPage, SavedSession, parseSessionSnapshot, recoverSessionSnapshot } from '@/lib/interview-session';
import {InterviewUsage,usageLabel} from '@/lib/interview-usage';
import { Input } from '@/components/ui/input';
import {MeetingControls, CallNotice, useMeetingMode} from '@/components/interview/MeetingMode';
import {addressedOnly} from '@/lib/meeting-mode';
import { RemoteUtteranceQueue, RemoteUtterance, TranscriptInput, UtteranceEnd, mergeSpokenTranscript, sameCapturedUtterance } from '@/lib/interview-scheduler';
import { useRecordingState } from '@/contexts/RecordingStateContext';
import { useTranscripts } from '@/contexts/TranscriptContext';
import { indexedDBService } from '@/services/indexedDBService';
import { LatencyRecord, TurnLatency, latencyLabel } from '@/lib/interview-latency';
import { AudioFault, InterviewAudioHealth } from '@/lib/interview-health';
import { MicrophoneState, MicrophoneTranscriptGate, stopInterviewAudio, listeningPhase } from '@/lib/interview-audio';
import { AudioLines, History as HistoryIcon, ChevronRight, Camera, AppWindow, Columns2, Play, Pause, Square, Search, Snowflake, FolderOpen, Settings2, FileText, Trash2 } from 'lucide-react';


type File = { path: string; bytes: number; eligible: boolean; reason: string };
type Choice = { path: string; kind: string; allow_external: boolean; structured?: StructuredSelection | null };
type Status = { cloud_asr_mode?:'local'|'aliyun'; workspace_ready?:boolean; key_configured: boolean; model?: string; answer_capacity?:number; vision_ready?:boolean; vision_model?:string; root: string; suggested_root: string; index_version: string; index_documents: number; index_chunks: number; documents: DocumentRecord[]; architecture: string; memory_bytes: number; system_version: string; aliases: string; facts: string; fact_card?: FactCard; whisper_model: string | null; session_active: boolean };
type ScreenDisplay={id:number;width:number;height:number;primary:boolean};

type ImportEvent = { import_id: string; phase: string; completed: number; total: number; path: string | null; report: unknown; message: string | null };
type ImportView = ImportEvent & { ready: boolean; cancelPending: boolean };
const importPhaseNames: Record<string,string> = { starting: '正在准备导入', scanning: '扫描目录', parsing: '解析文件', parsed: '文件已解析', indexing: '更新索引', cancelling: '正在取消导入，保留原资料库…', committing: '正在完成保存' };
type Stream = { question_id: string; revision: number; event: { type: string; attempt?: number; text?: string; message?: string; evidence?: Source[]; conflicts?: SourceConflict[]; retrieval_ms?: number; fact_version?: number; first_text_ms?: number; elapsed_ms?: number; usage?: { total_tokens?: number } | null } };
const DEFAULT_REQUEST_LIMIT = 80;
const REQUEST_LIMIT_WARNING = 5;
const kindNames: Record<string, string> = { personal: '个人事实汇总', evidence: '原始证据', practice: '练习参考', job: '岗位信息', technical: '技术笔记' };

export default function InterviewPage() {
  const recording = useRecordingState();
  const { resetAfterHistoryClear } = useTranscripts();
  const [status, setStatus] = useState<Status | null>(null);
  const [question, setQuestion] = useState('');
  const [asrCloudAllowed,setAsrCloudAllowed]=useState(false);
  const [answer, setAnswer] = useState('');
  const [sources, setSources] = useState<Source[]>([]);
  const [conflicts, setConflicts] = useState<SourceConflict[]>([]);
  const [phase, setPhase] = useState('就绪');
  const [notice, setNotice] = useState('');
  const [answerWarning,setAnswerWarning] = useState('');
  const [compactControls,setCompactControls]=useState(false);
  const [settings, setSettings] = useState(false);
  const [settingsTab, setSettingsTab] = useState<'资料库'|'事实与术语'|'模型接口'|'音频'>('资料库');
  const [openBackground,setOpenBackground]=useState(false);
  const [openVision, setOpenVision] = useState(false);
  const settingsTrigger = useRef<HTMLButtonElement>(null);
  const [operationBusy, setOperationBusy] = useState(false);
  const [connectionChecking,setChecking]=useState(false);
  const cancellation=useRef<Promise<void>|null>(null);
  const [cancellationPending,setCancellationPending]=useState(false);
  // Block shortcuts and queue consumption immediately, before React renders
  // the disabled controls for an operation or connection check.
  const operationBusyRef=useRef(false),connectionCheckingRef=useRef(false);
  const setBusy=useCallback((value:boolean)=>{operationBusyRef.current=value;setOperationBusy(value);},[]);
  const setConnectionChecking=useCallback((value:boolean)=>{connectionCheckingRef.current=value;setChecking(value);},[]);
  const [connectionRevision,setConnectionRevision]=useState(0);
  const busy=operationBusy||connectionChecking||cancellationPending;
  const [generating, setGenerating] = useState(false);
  const [frozen, setFrozen] = useState(false);
  const [automatic, setAutomatic] = useState(false);
  const resumeAutomatic = useRef(false);
  const [queued, setQueued] = useState(0);
  const [audioAllowed, setAudioAllowed] = useState(false);
  const [cloudAllowed, setCloudAllowed] = useState(false);
  const [transcripts, setTranscripts] = useState<TranscriptInput[]>([]);
  const [root, setRoot] = useState('');
  const [files, setFiles] = useState<File[]>([]);
  const [filter, setFilter] = useState('');
  const [selected, setSelected] = useState<Record<string, Choice>>({});
  const incompleteStructured = Object.values(selected).some(s => isStructuredPath(s.path) && !s.structured?.pointers.length);
  const [aliases, setAliases] = useState('');
  const [key, setKey] = useState('');
  const [history, setHistory] = useState<Turn[]>([]);
  const historyRef = useRef<Turn[]>([]);
  historyRef.current = history;
  const clearingHistory = useRef(false);
  const acceptTranscripts = useRef(true);
  const [calls, setCalls] = useState(0);
  const [residueRevision,setResidueRevision]=useState(0);
  const [initialMeetingId,setInitialMeetingId]=useState<string>();
  const [savedSessions, setSavedSessions] = useState<SavedSession[] | null>(null);
  const [allowReplaceSession, setAllowReplaceSession] = useState(false);
  const [savedBefore, setSavedBefore] = useState<number | null>(null);
  const sessionOperation = useRef(false);
  const [limit, setLimit] = useState(DEFAULT_REQUEST_LIMIT);
  const [feedGeneration,setFeedGeneration]=useState(0);
  const [editingSelfId, setEditingSelfId] = useState<string | null>(null);
  const selfEditBase=useRef<{id:string;text:string}|null>(null);
  const spoken = useRef<TranscriptInput[]>([]);
  const captureSession = useRef(0);
  const [timing, setTiming] = useState('');
  const [usage, setUsage] = useState<string>('未知');
  const usageLedger=useRef(new InterviewUsage());
  const [,setUsageVersion]=useState(0);
  const [report, setReport] = useState('');
  const [importJob, setImportJob] = useState<ImportView | null>(null);
  const importId = useRef<string | null>(null);
  const clearingIndex = useRef(false);
  const [level, setLevel] = useState(0);
  const [micLevel, setMicLevel] = useState(0);
  const [recordMicrophone, setRecordMicrophone] = useState(true);
  useEffect(()=>{try{setRecordMicrophone(localStorage.getItem('tingxian-microphone')!=='off');}catch{}},[]);
  const microphoneGate = useRef(new MicrophoneTranscriptGate());
  const microphoneChanging = useRef(false);
  const microphoneVersion = useRef(0);
  const audioHealth = useRef(new InterviewAudioHealth());
  const [healthView, setHealthView] = useState(() => audioHealth.current.view());
  const healthPauseApplied = useRef(false);
  const healthResumeAutomatic = useRef(false);
  const [confirmedAnswer, setConfirmedAnswer] = useState('');
  const requests=useRef(new AnswerRequests());
  const capacity=useRef(1);
  const [screenDisplays,setScreenDisplays]=useState<ScreenDisplay[]>([]);
  const [screenId,setScreenId]=useState<number|null>(null);
  const [preferences,setPreferences]=useState<PresentationPreferences>(presentationDefaults);
  const [split,setSplit]=useState(false),[windowChanging,setWindowChanging]=useState(false);
  const presentationEpoch=useRef(''),presentationVersion=useRef(0);
  const presentationRef=useRef<unknown>(null);
  const presentationDirty=useRef(false);
  useEffect(()=>{try{setPreferences(readPresentationPreferences(JSON.parse(localStorage.getItem('interview-presentation')||'null')));}catch{}},[]);
  const changePreferences=(next:PresentationPreferences)=>{setPreferences(next);localStorage.setItem('interview-presentation',JSON.stringify(next));};
  const current = useRef({ question: '', answer: '' });
  const utterances = useRef(new RemoteUtteranceQueue());
  const meeting = useMeetingMode(()=>{
    const pending=new Set(utterances.current.list().map(turn=>turn.id));
    utterances.current.clear();setQueued(0);
    setHistory(rows=>rows.map(row=>pending.has(row.id)?{...row,status:'recorded'}:row));
  });
  const latencies = useRef(new Map<string, TurnLatency>());
  const live = useRef({ automatic, frozen, cloudAllowed, calls, limit });
  live.current = { automatic, frozen, cloudAllowed, calls, limit };
  const submitRef = useRef<(turn: RemoteUtterance) => void>(() => {});
  const error = useCallback((e: unknown) => setNotice(String(e)), []);
  const syncHealth = useCallback(() => {
    const view = audioHealth.current.view();
    setHealthView(previous => previous.paused === view.paused && previous.message === view.message
      && previous.canResume === view.canResume && previous.advisory === view.advisory ? previous : view);
    if (view.paused) {
      if (!healthPauseApplied.current) healthResumeAutomatic.current = live.current.automatic;
      // Synchronous guard closes the timer race before React renders.
      live.current.automatic = false; setAutomatic(false);
      if (!healthPauseApplied.current) {
        const pending = new Set(utterances.current.list().map(t => t.id));
        for (const id of pending) latencies.current.get(id)?.end('cancelled');
        utterances.current.clear(); setQueued(0);
        setHistory(h => h.map(t => pending.has(t.id) || t.status === 'listening' ? { ...t, status: 'recorded', latency: latencies.current.get(t.id)?.snapshot() || t.latency } : t));
      }
    }
    healthPauseApplied.current = view.paused;
  }, []);
  const applyMicrophoneState = useCallback((state: MicrophoneState) => {
    if (state.health && state.recording && !audioHealth.current.observe(state.health)) return;
    microphoneGate.current.apply(state);
    if (state.recording) setRecordMicrophone(state.enabled);
    if (!state.enabled) setMicLevel(0);
    syncHealth();
  }, [syncHealth]);


  const refresh = useCallback(async () => {
    const s = await invoke<Status>('interview_status');
    capacity.current=s.answer_capacity??1;setStatus({...s,key_configured:s.workspace_ready??s.key_configured}); setRoot(current => current || s.root || s.suggested_root || ""); setAliases(s.aliases);
  }, []);
  const refreshDocuments = useCallback(async () => {
    // Reviewing a document must not overwrite unsaved facts, aliases or the
    // next import's selected directory with the persisted settings.
    const s=await invoke<Status>('interview_status');setStatus({...s,key_configured:s.workspace_ready??s.key_configured});
  }, []);
  const applyImportEvent = useCallback((event: ImportEvent | null, adopt = false) => {
    if (!event) return;
    const terminal = ['completed', 'cancelled', 'failed'].includes(event.phase);
    if (!importId.current && adopt && !terminal) importId.current = event.import_id;
    if (event.import_id !== importId.current) return;
    if (terminal) {
      importId.current = null; setImportJob(null); setBusy(false);
      if (event.phase === 'completed') { setReport(JSON.stringify(event.report, null, 2)); setNotice('资料导入已完成'); void refresh().catch(error); }
      else { setNotice(event.message || '导入未完成'); setReport(event.message || '导入未完成'); }
    } else {
      setBusy(true);
      setImportJob(previous => ({ ...event, ready: true, cancelPending: event.phase === 'cancelling' || (previous?.import_id === event.import_id && previous.cancelPending) }));
    }
  }, [refresh, error]);
  const startImport = async () => {
    if (importId.current) return;
    if (incompleteStructured) { setNotice('请先预览并明确选择 JSON/YAML 的字段或整份文件。'); return; }
    const id = crypto.randomUUID(); importId.current = id;
    setBusy(true); setNotice(''); setReport('');
    setImportJob({ import_id: id, phase: 'starting', completed: 0, total: Object.keys(selected).length, path: null, report: null, message: null, ready: false, cancelPending: false });
    try {
      await invoke('profile_import', { importId: id, root, selected: Object.values(selected) });
      if (importId.current === id) setImportJob(job => job && { ...job, ready: true });
    } catch (e) {
      if (importId.current === id) { importId.current = null; setImportJob(null); setBusy(false); error(e); }
    }
  };
  const cancelImport = async () => {
    const id = importId.current; if (!id) return;
    setImportJob(job => job && { ...job, cancelPending: true });
    try {
      const status = await invoke<'requested' | 'committing' | 'finished'>('profile_import_cancel', { importId: id });
      if (importId.current !== id) return;
      if (status === 'committing') setImportJob(job => job && { ...job, phase: 'committing', cancelPending: false });
      // Reconcile even when completion races the click or an event was missed.
      applyImportEvent(await invoke<ImportEvent | null>('profile_import_status'));
    } catch (e) { if (importId.current === id) { setImportJob(job => job && { ...job, cancelPending: false }); error(e); } }
  };
  const cancel = useCallback(() => {
    if(cancellation.current)return cancellation.current;
    const running=requests.current.clear();
    const cancelledIds=new Set(running.map(j=>j.id));
    for(const job of running)usageLedger.current.finish(job.id,job.revision,'cancelled');
    setUsageVersion(v=>v+1);
    const pendingIds=new Set(utterances.current.list().map(t=>t.id));
    for(const id of [...cancelledIds,...pendingIds])latencies.current.get(id)?.end('cancelled');
    setHistory(h=>h.map(t=>({...t,latency:latencies.current.get(t.id)?.snapshot()||t.latency,
      status:cancelledIds.has(t.id)?'cancelled':pendingIds.has(t.id)?'recorded':t.status})));
    utterances.current.clear();setQueued(0);setGenerating(false);setPhase('已取消 · 保留部分答案');
    setCancellationPending(true);
    const pending=invoke<void>('answer_cancel').catch(e=>{
      live.current.automatic=false;setAutomatic(false);throw e;
    }).finally(()=>{cancellation.current=null;setCancellationPending(false);});
    cancellation.current=pending;return pending;
  }, []);
  const submit = useCallback(async (q: string, automated = false, turnId?: string, screenshot = false) => {
    if(interviewExit.pending())return;
    if (operationBusyRef.current || connectionCheckingRef.current || cancellation.current || importId.current || clearingIndex.current || sessionOperation.current) { setNotice('请等待当前操作完成后再提交。'); return; }
    if (!q.trim()) return;
    if (editingSelfId && !automated && !screenshot) {
      const turn = historyRef.current.find(t => t.id === editingSelfId && t.role==='self');
      if(!turn || selfEditBase.current?.id!==turn.id || selfEditBase.current.text!==turn.question){
        setNotice('这条本人回答已更新或被移除；当前修正文字已保留，请重新选择要修正的回答。');return;
      }
      spoken.current = spoken.current.map(t => t.source==='Microphone' && t.sequence_id === turn.sequence_id && t.capture_session === turn.capture_session ? { ...t, text: q } : t);
      setHistory(h => h.map(t => t.id === editingSelfId ? { ...t, question: q } : t));
      selfEditBase.current=null;setEditingSelfId(null); setQuestion('');setNotice('本人回答已修正，未调用 AI。');return;
    }
    if (!cloudAllowed) { setNotice('请先勾选允许将对方发言、双方近期对话与选定证据发送给已配置的 AI 接口。'); return; }
    if(screenshot&&!status?.vision_ready){setSavedSessions(null);setSettingsTab('模型接口');setOpenBackground(false);setOpenVision(true);setSettings(true);setNotice('请先在“截屏视觉模型”中配置支持图片的接口、模型和 Key。尚未截屏或发送。');return;}
    const reserved=usageLedger.current.snapshot().requests.filter(r=>!r.accepted&&r.state==='pending').length;
    if (usageLedger.current.acceptedCalls()+reserved >= live.current.limit) { setNotice('已达到本次会话请求软上限，可手动增加 20 次。'); return; }
    const id=turnId||crypto.randomUUID();
    const revision=requests.current.begin(id,screenshot?1:capacity.current,screenshot?'vision':'text');
    if(revision===null){setNotice('回答通道正在使用，请稍后提交；当前输入保留。');return;}
    setFrozen(false);
    usageLedger.current.begin(id,revision);setUsageVersion(v=>v+1);
    const trace = latencies.current.get(id) || new TurnLatency();
    trace.mark('submitted'); latencies.current.set(id, trace);
    current.current = { question: q, answer: '' };
    setHistory(h => {
      const next: Turn = { meeting_mode:meeting.current.current.mode, role: automated ? 'remote' : 'manual', id, question: q, suggestion: '', confirmed_answer: null, status: 'generating', sources: [], time: new Date().toLocaleTimeString('zh-CN'), timing: latencyLabel(trace.snapshot()), latency: trace.snapshot() };
      return h.some(t=>t.id===id)?h.map(t=>t.id===id?{...next,time:t.time}:t):[...h,next];
    });
    if (!automated) setQuestion('');
    setAnswer(''); setSources([]); setConflicts([]); setNotice(''); setAnswerWarning(''); setTiming(''); setUsage('未知'); setGenerating(true); setPhase('检索与生成中');
    try {
      await invoke(screenshot?'interview_screenshot_answer_start':'answer_start', { answerFirst:true, questionId: id, revision, meetingMode:meeting.current.current.mode, question: q, history: historyRef.current.filter(t => t.status === 'done' && t.role !== 'self').slice(-3).map(t => ({ question: answerPresentation(t.suggestion,true,t.question).question.slice(0,1500), confirmed_answer: t.confirmed_answer })), dialogue: spoken.current.slice(-12).map(t => ({ source: t.source, text: t.text })),...(screenshot?{displayId:screenId}:{}) });
      if(usageLedger.current.accept(id,revision)){setCalls(n=>n+1);setUsageVersion(v=>v+1);}
      if (requests.current.get(id,revision)) setStatus(s => s ? { ...s, session_active: true } : s);
    } catch (e) {
      if(usageLedger.current.finish(id,revision,'error'))setUsageVersion(v=>v+1);
      if(requests.current.finish(id,revision)){
        trace.end('error');setHistory(h=>h.map(t=>t.id===id?{...t,status:'error',latency:trace.snapshot()}:t));
        error(e);setGenerating(requests.current.size>0);setPhase('本题请求失败 · 其他回答继续');
      }
    }
  }, [cloudAllowed, calls, limit, history, editingSelfId, error, status?.vision_ready, screenId]);
  submitRef.current = turn => void submit(turn.text, true, turn.id);

  const editTurn=useCallback((id:string)=>{
    const turn=historyRef.current.find(t=>t.id===id);if(!turn)return;
    setQuestion(turn.status==='done'&&turn.role!=='self'?answerPresentation(turn.suggestion,true,turn.question).question:turn.question);selfEditBase.current=turn.role==='self'?{id:turn.id,text:turn.question}:null;
    setEditingSelfId(turn.role==='self'?turn.id:null);
  },[]);
  useEffect(()=>{
    if(!presentationEpoch.current)presentationEpoch.current=crypto.randomUUID();
    presentationRef.current={epoch:presentationEpoch.current,version:++presentationVersion.current,
      call_notice:meeting.notice,history:history.map(({id,role,question,suggestion,status,time,passive})=>({id,role,question,suggestion,status,time,passive})),preferences,selected_id:null};
    presentationDirty.current=true;
  },[history,preferences,meeting.notice]);
  useEffect(()=>{
    if(!isTauri())return;
    let disposed=false;const stops:UnlistenFn[]=[];
    const add=async<T,>(name:string,receive:(value:T)=>void)=>{const off=await listen<T>(name,e=>receive(e.payload));if(disposed)off();else stops.push(off);};
    void add<boolean>('interview-window-mode',value=>{setSplit(value);presentationDirty.current=true;}).catch(error);
    void add<{type:string;id:string|null}>('interview-presentation-action',action=>{
      if(action.type==='edit'&&action.id)editTurn(action.id);
    }).catch(error);
    void invoke<boolean>('interview_window_mode_get').then(value=>{if(!disposed)setSplit(value);}).catch(error);
    return()=>{disposed=true;stops.forEach(off=>off());};
  },[editTurn,error]);
  useEffect(()=>{
    if(!split||!isTauri())return;
    presentationDirty.current=true;let sending=false;
    const publish=()=>{if(sending||!presentationDirty.current||!presentationRef.current)return;
      sending=true;presentationDirty.current=false;
      void invoke('interview_presentation_publish',{snapshot:presentationRef.current}).catch(error).finally(()=>{sending=false;});
    };
    publish();const timer=setInterval(publish,80);return()=>clearInterval(timer);
  },[split,error]);

  useEffect(()=>{
    let handled:number|null=null;
    const freeze=()=>{
      const id=interviewExit.getSnapshot().requestId;
      if(id===null || handled===id)return;handled=id;
      live.current.automatic=false;setAutomatic(false);
      void cancel().catch(error);
      setPhase('正在退出 · 自动回答已暂停');
    };
    const stop=interviewExit.subscribe(freeze);freeze();return stop;
  },[cancel,error]);

  useEffect(() => {
    if (!isTauri()) { setNotice('此页面需在听弦桌面应用运行。浏览器预览不能调用本地资料、音频或 API。'); return; }
    void refresh().catch(error);
    let disposed = false; const unlisten: UnlistenFn[] = [];
    const add = async <T,>(name: string, fn: (payload: T) => void) => {
      const stop = await listen<T>(name, e => fn(e.payload));
      if (disposed) stop(); else unlisten.push(stop);
    };
    void add<ImportEvent>('profile-import', event => applyImportEvent(event)).catch(error);
    void invoke<ImportEvent | null>('profile_import_status').then(event => { if (!disposed) applyImportEvent(event, true); }).catch(error);
    void add<Stream>('interview-answer', ({ question_id, revision, event }) => {
      // Late usage may improve accounting, but must never update another answer.
      if(usageLedger.current.event(question_id,revision,event)){
        // Native events can arrive before answer_start's acknowledgement.
        if(usageLedger.current.accept(question_id,revision))setCalls(n=>n+1);
        setUsageVersion(v=>v+1);
      }
      if (!requests.current.get(question_id,revision)) return;
      const trace = latencies.current.get(question_id);
      trace?.provider(event);
      if (event.type === 'sources') trace?.mark('retrieval_done');
      if (event.type === 'request_sent') trace?.mark('request_sent');
      if (event.type === 'sources') { setSources(event.evidence || []); setConflicts(event.conflicts || []); setHistory(h => h.map(t => t.id === question_id ? { ...t, sources: event.evidence || [], conflicts: event.conflicts || [], fact_version: event.fact_version } : t)); }
      if (event.type === 'delta') {
        const text=requests.current.append(question_id,revision,event.text||'')!;
        current.current={question:historyRef.current.find(t=>t.id===question_id)?.question||'',answer:text};setAnswer(text);
        trace?.observe(text);if(trace?.snapshot().stages_ms.first_useful_text!==undefined)setAnswerWarning('');
        setHistory(h=>h.map(t=>t.id===question_id?{...t,suggestion:text}:t));setPhase(`正在回答 · ${requests.current.size} 路`);
      }
      if (event.type === 'warning') setAnswerWarning(event.message || '');
      if(event.type==='usage'){
        const total=event.usage?.total_tokens;
        if(typeof total==='number' && Number.isSafeInteger(total) && total>=0)setUsage(`${total} tokens`);
      }
      if(event.type==='error'){
        trace?.end('error');requests.current.finish(question_id,revision);setGenerating(requests.current.size>0);
        setHistory(h=>h.map(t=>t.id===question_id?{...t,status:'error'}:t));setNotice(event.message||'本题请求失败');setPhase('本题未完成 · 其他回答继续');
      }
      if (event.type === 'done') {
        requests.current.finish(question_id,revision);setGenerating(requests.current.size>0);setPhase(requests.current.size?`正在回答 · ${requests.current.size} 路`:'回答已就绪');trace?.end('done');
        const total=event.usage?.total_tokens;
        setUsage(typeof total==='number' && Number.isSafeInteger(total) && total>=0 ? `${total} tokens` : '未知');
        setHistory(h => h.map(t => t.id === question_id ? { ...t, status: 'done' } : t));
      }
      if (trace) {
        const latency = trace.snapshot(); const label = latencyLabel(latency);
        setTiming(label); setHistory(h => h.map(t => t.id === question_id ? { ...t, latency, timing: label } : t));
      }
    }).catch(error);
    void add<TranscriptInput>('transcript-update', incoming => {
      if (clearingHistory.current || !acceptTranscripts.current || !audioHealth.current.accepts(incoming.native_session_id)) return;
      const event = { ...incoming, capture_session: captureSession.current };
      if (!microphoneGate.current.accepts(event)) return;
      if (event.is_partial) {
        setTranscripts(previous=>previous.some(t=>sameCapturedUtterance(t,event)&&(!t.is_partial||t.interrupted))
          ? previous : [...previous.filter(t=>!sameCapturedUtterance(t,event)),event].slice(-60));
        return;
      }
      setTranscripts(previous => [...previous.filter(t => (t.sequence_id !== event.sequence_id || t.capture_session !== event.capture_session) && !(t.is_partial && sameCapturedUtterance(t,event))), event].slice(-60));
      spoken.current = mergeSpokenTranscript(spoken.current, event);
      if (event.source === 'Microphone') {
        meeting.interrupt();
        setHistory(h => [...h.filter(t => t.id !== `mic-${event.sequence_id}` && !(event.replaces_utterance && t.role === 'self' && sameCapturedUtterance({ ...t, source: 'Microphone' }, event))), { role: 'self', sequence_id: event.sequence_id, utterance_id: event.utterance_id, microphone_epoch: event.microphone_epoch, capture_session: event.capture_session, id: `mic-${event.sequence_id}`, question: event.text, suggestion: '', confirmed_answer: null, status: 'done', sources: [], time: event.timestamp, timing: event.replaces_utterance ? '已用整段语音复核' : '' }]);
        return;
      }
      audioHealth.current.transcript();
      const draft = utterances.current.receive(event);
      if (draft) setHistory(h => {
        const row: Turn = { passive:addressedOnly(meeting.current.current.mode), meeting_mode:meeting.current.current.mode, role: 'remote', id: draft.id, question: draft.text, suggestion: '', confirmed_answer: null, status: audioHealth.current.paused ? 'recorded' : 'listening', sources: [], time: draft.time, timing: '' };
        return h.some(t => t.id === row.id) ? h.map(t => t.id === row.id ? row : t) : [...h, row];
      });
    }).catch(error);
    void add<UtteranceEnd>('interview-utterance-end', event => {
      if (clearingHistory.current || !acceptTranscripts.current || !audioHealth.current.accepts(event.native_session_id)) return;
      setTranscripts(previous=>previous.map(t=>t.is_partial && t.source===event.source && t.utterance_id===event.utterance_id && t.native_session_id===event.native_session_id?{...t,interrupted:true}:t));
      const l = live.current;
      const result = utterances.current.finish(event, l.automatic && !l.frozen && l.cloudAllowed && !audioHealth.current.paused, meeting.decide);
      if (result.turn) {
        const trace = new TurnLatency(performance.now(), event.success ? event.timing : null, Date.now(), event.success);
        latencies.current.set(result.turn.id, trace);
        if (result.status === 'failed') trace.end('error');
        setHistory(h => h.map(t => t.id === result.turn!.id ? { ...t, passive: t.passive && result.status!=='queued', latency: trace.snapshot(), status: result.status === 'queued' ? 'queued' : 'recorded' } : t));
      }
      if (result.status === 'full' || result.status === 'failed') {
        live.current.automatic = false; setAutomatic(false);
        setNotice(result.status === 'full' ? '待处理超过 20 段或整段发言超过 1500 字，已暂停自动发送；原文保留，可手动分段处理。' : '这段对方发言存在转写失败，已保留可用文字并暂停自动回答，请核对后重试。');
      }
      setQueued(utterances.current.size);
    }).catch(error);
    void add<AudioFault>('interview-audio-fault', event => { if (audioHealth.current.fault(event)) syncHealth(); }).catch(error);
    void add<{native_session_id:number;message:string}>('interview-transcription-cancelled', event => {
      if (!clearingHistory.current && audioHealth.current.accepts(event.native_session_id)) {
        setNotice(event.message); setHistory(h => h.map(t => t.status === 'listening' ? {...t,status:'recorded'} : t));
      }
    }).catch(error);
    // Modern faults carry a recording ID. Unscoped compatibility events from
    // an older worker must not pause a newly started recording.
    void add<{ native_session_id?: number; userMessage?: string }>('transcription-error', event => { if (audioHealth.current.accepts(event.native_session_id)) { live.current.automatic = false; setAutomatic(false); setNotice(event.userMessage || '转写失败，已暂停自动发送'); } }).catch(error);
    void add<string>('recording-error', message => { if (audioHealth.current.accepts()) { live.current.automatic = false; setAutomatic(false); setNotice(message || '采集失败，已暂停自动发送'); } }).catch(error);
    void add<MicrophoneState>('interview-microphone-state', state => {
      if (!microphoneChanging.current && (!state.health || audioHealth.current.accepts(state.health.session_id))) applyMicrophoneState(state);
    }).catch(error);
    let pollingLevel = false;
    const levelTimer = setInterval(() => {
      audioHealth.current.checkPoll(); syncHealth();
      if (importId.current) void invoke<ImportEvent | null>('profile_import_status').then(event => { if (!disposed) applyImportEvent(event); }).catch(() => {});
      if (microphoneChanging.current || pollingLevel) return;
      pollingLevel = true;
      const version = microphoneVersion.current;
      void invoke<{system:number;microphone:number;microphone_state:MicrophoneState}>('interview_audio_level').then(levels => {
        if (disposed || microphoneChanging.current || version !== microphoneVersion.current) return;
        setLevel(levels.system); setMicLevel(levels.microphone_state.enabled ? levels.microphone : 0); applyMicrophoneState(levels.microphone_state);
      }).catch(() => {}).finally(() => { pollingLevel = false; });
    }, 500);
    const timer = setInterval(() => {
      const l = live.current;
      if (interviewExit.pending() || operationBusyRef.current || connectionCheckingRef.current || cancellation.current || !l.automatic || l.frozen || !l.cloudAllowed || requests.current.size>=3 || requests.current.count('text')>=capacity.current || audioHealth.current.paused) return;
      const reserved=usageLedger.current.snapshot().requests.filter(r=>!r.accepted&&r.state==='pending').length;
      if (usageLedger.current.acceptedCalls()+reserved >= l.limit) {
        if (utterances.current.size) { setAutomatic(false); setNotice('已达到会话请求上限；增加额度后可重新勾选自动发送，继续待处理发言。'); }
        return;
      }
      const turn = utterances.current.take();
      if (turn) { setQueued(utterances.current.size); submitRef.current(turn); }
    }, 100);
    return () => { disposed = true; unlisten.forEach(stop => stop()); clearInterval(timer); clearInterval(levelTimer); requests.current.clear(); void invoke('answer_cancel').catch(() => {}); };
  }, [refresh, error, applyMicrophoneState, applyImportEvent, syncHealth]);

  useEffect(() => {
    if (!recording.isActive) setAutomatic(false);
  }, [recording.isActive]);
  const run = async (work: () => Promise<unknown>) => { setBusy(true); setNotice(''); try { await work(); } catch (e) { error(e); } finally { setBusy(false); } };
  const localSearch = () => run(async () => { setAutomatic(false); await cancel(); const result=await invoke<{evidence:Source[];conflicts:SourceConflict[]}>('rag_search', { question }); setSources(result.evidence); setConflicts(result.conflicts); setPhase('仅本地检索 · 未请求模型'); });
  const end = () => run(async () => {
    setAutomatic(false); live.current.automatic = false; await cancel();
    await stopInterviewAudio();
    microphoneGate.current.close(); audioHealth.current.close(); syncHealth(); setHistory(h => h.map(t => t.status === 'listening' ? {...t,status:'recorded'} : t)); setMicLevel(0); setLevel(0);
    await invoke('session_clear');
    setQuestion(''); setConfirmedAnswer(''); meeting.reset(); utterances.current.reset(); setQueued(0);
    await saveSessionSnapshot();
    current.current = { question: '', answer: '' }; setPhase('会话已结束 · 已保存到本机，可恢复'); await refresh();
  });
  const sessionSnapshot = (): SessionSnapshot => parseSessionSnapshot({version:1,history:historyRef.current,transcripts,spoken_dialogue:spoken.current,current:current.current,calls:usageLedger.current.acceptedCalls(),usage,usage_ledger:usageLedger.current.snapshot()});
  const draftRecovery = useInterviewRecovery(()=>({version:1,snapshot:parseSessionSnapshot({version:1,history:historyRef.current,transcripts,spoken_dialogue:spoken.current,current:current.current,calls:usageLedger.current.acceptedCalls(),usage,usage_ledger:usageLedger.current.snapshot()},true),composer:{question,confirmed_answer:confirmedAnswer}}));
  const saveSessionSnapshot = async () => {
    const saved = await invoke<SavedSession>('interview_session_save',{snapshot:sessionSnapshot(),confirmed:true});
    setSavedSessions(null);setSavedBefore(null);
    setNotice(`已保存到本机（${new Date(saved.saved_at).toLocaleString('zh-CN')}，${saved.turns} 条发言）。之后的新内容需再次保存；重启后可点击“查看已保存会话”。`);
  };
  const listSessions = (more=false) => run(async()=>{
    if(!more)setSavedSessions([]);
    const page=await invoke<SessionPage>('interview_session_list',{before:more?savedBefore:null});
    setSavedSessions(old=>more?[...(old || []),...page.sessions]:page.sessions);setSavedBefore(page.next_before);
  });
  useEffect(()=>{
    if(typeof window==='undefined'||!window.location)return;
    const q=new URLSearchParams(window.location.search),panel=q.get('panel');
    if(panel==='audio'){setSettingsTab('音频');setSettings(true);}
    if(panel==='history'){setInitialMeetingId(q.get('meeting')||undefined);void listSessions();}
  },[]);
  const restoreSession = (id:number|string) => run(async()=>{
    if (sessionOperation.current || recording.isRecording || requests.current.size>0) throw Error('请先结束当前采集和回答，再恢复保存记录');
    if ((historyRef.current.length || question || confirmedAnswer) && !allowReplaceSession) throw Error('请先保存当前内容，再明确允许替换当前显示');
    const previousHistory=historyRef.current;
    sessionOperation.current=true;acceptTranscripts.current=false;
    try {
      const draft=typeof id==='string'?parseRecoverySnapshot(await invoke<RecoverySnapshot>('interview_recovery_load',{id})):null;
      const saved=recoverSessionSnapshot(draft?.snapshot ?? await invoke<unknown>('interview_session_load',{id}),crypto.randomUUID(),!!draft);
      if (historyRef.current !== previousHistory) throw Error('当前内容发生变化，未替换；请重新选择恢复');
      setAllowReplaceSession(false);
      live.current.automatic=false;setAutomatic(false);setAudioAllowed(false);setAsrCloudAllowed(false);setCloudAllowed(false);setFrozen(false);
      requests.current.clear();meeting.reset(); utterances.current.reset();latencies.current.clear();setQueued(0);setGenerating(false);
      microphoneGate.current.close();audioHealth.current.close();syncHealth();
      captureSession.current=Math.max(0,...saved.spoken_dialogue.map(t=>t.capture_session || 0),...saved.history.map(t=>t.capture_session || 0))+1;
      historyRef.current=saved.history;setHistory(saved.history);setTranscripts(saved.transcripts);spoken.current=saved.spoken_dialogue;
      presentationEpoch.current=crypto.randomUUID();setFeedGeneration(v=>v+1);
      current.current=saved.current;setCalls(saved.calls);setUsage(saved.usage);setLimit(Math.max(DEFAULT_REQUEST_LIMIT,saved.calls));
      usageLedger.current=new InterviewUsage(saved.usage_ledger??undefined,saved.calls);setUsageVersion(v=>v+1);
      setQuestion(draft?.composer.question||'');setAnswer('');setSources([]);setConflicts([]);setConfirmedAnswer(draft?.composer.confirmed_answer||'');setEditingSelfId(null);setTiming('');
      selfEditBase.current=null;
      setSavedSessions(null);setSavedBefore(null);setCompactControls(false);
      setPhase(draft?'已恢复草稿 · 采集和自动回答均未开启':'已恢复保存会话 · 采集和自动回答均未开启');setNotice('已载入保存时的双方发言、AI建议和来源。旧来源与事实版本仅供回顾；不会自动补发未完成问题。继续前请重新确认转写和外发许可。');
    } finally {sessionOperation.current=false;}
  });
  const exportSession = () => run(async () => {
    const snapshot = { exported_at: new Date().toISOString(), latency_notes: '各阶段为稳定转写到达界面（手动为提交时刻）起的毫秒数；发声结束为VAD估计，非人工标注。native到界面使用系统时钟桥接，其余界面阶段使用单调时钟；无效桥接或强制停止时缺失，不补零。request_sent为界面收到原生发送事件的时刻；首句为建议回答中首个句末标点到达，不等于人工语义验收。', mode: 'AI 建议不等于本人实际回答', spoken_dialogue: spoken.current, transcripts, history, current: current.current, calls:usageLedger.current.acceptedCalls(), usage, usage_ledger:usageLedger.current.snapshot() };
    const result = await invoke<{content:string;credentials_redacted:boolean}>('interview_session_export',{snapshot});
    const url = URL.createObjectURL(new Blob([result.content], { type: 'application/json' }));
    try {const a = document.createElement('a'); a.href = url; a.download = 'interview-session.json'; a.click();}
    finally {URL.revokeObjectURL(url);}
    if(result.credentials_redacted)setNotice('已导出本次记录，其中的 API 凭证已隐藏。');
  });
  const clearIndex = () => {
    if (clearingIndex.current) return;
    clearingIndex.current = true;
    return run(async () => {
    try {
      const result = await invoke<{archive_path: string | null; documents: number; chunks: number; metadata_records?: number; version: number}>('profile_index_clear');
      setSources([]); setConflicts([]); setReport(JSON.stringify(result, null, 2));
      setNotice(result.archive_path ? `已清空 ${result.documents} 份资料、${result.chunks} 个索引片段、${result.metadata_records || 0} 条资料核对记录。备份在废纸篓：${result.archive_path}。原文件、事实卡与会话历史保留。` : '资料索引已为空；没有新增废纸篓备份。');
      await refresh();
    } finally { clearingIndex.current = false; }
    });
  };
  const clearHistory = () => run(async () => {
    if (clearingHistory.current) return;
    clearingHistory.current = true; acceptTranscripts.current = false;
    live.current.automatic = false; setAutomatic(false);
    indexedDBService.suspendRecordingWrites();
    setPhase('正在停止采集并将历史移入废纸篓');
    await draftRecovery.writer.pause();
    let archivePath = '';
    try {
      await cancel();
      const capture = await invoke<{is_recording: boolean}>('get_recording_state');
      if (capture.is_recording) await stopInterviewAudio();
      microphoneGate.current.close(); audioHealth.current.close(); syncHealth();
      const recovery = await indexedDBService.snapshotInterviewHistory();
      const report = await invoke<{archive_path:string; recording_folders:number; database_meetings:number; saved_sessions?:number}>('interview_history_clear', {
        snapshot: { current_session: { composer:{question,confirmed_answer:confirmedAnswer}, history: historyRef.current, transcripts, spoken_dialogue: spoken.current, current: current.current, calls:usageLedger.current.acceptedCalls(), usage, usage_ledger:usageLedger.current.snapshot() }, recovery }
      });
      archivePath = report.archive_path;
      await indexedDBService.removeArchivedInterviewHistory(recovery);
      resetAfterHistoryClear();
      meeting.reset(); utterances.current.reset(); latencies.current.clear(); spoken.current = []; historyRef.current = [];
      requests.current.clear(); current.current = { question:'', answer:'' };
      setSavedSessions(null);setSavedBefore(null);
      setHistory([]); setTranscripts([]); setQuestion(''); setAnswer(''); setSources([]); setConflicts([]);
      setConfirmedAnswer(''); setEditingSelfId(null); setQueued(0); setCalls(0); setLimit(DEFAULT_REQUEST_LIMIT);
      selfEditBase.current=null;
      usageLedger.current=new InterviewUsage();setUsageVersion(v=>v+1);
      setTiming(''); setUsage('未知'); setGenerating(false); setFrozen(false); setLevel(0); setMicLevel(0);
      presentationEpoch.current=crypto.randomUUID();setFeedGeneration(v=>v+1);
      await draftRecovery.writer.resetAfterClear();
      setPhase('历史已清空 · 可重新开始');
      setNotice(`历史已移入废纸篓：${report.recording_folders} 个录音记录目录、${report.database_meetings} 条已保存会议、${report.saved_sessions || 0} 份会话快照，以及本次对话和恢复缓存。个人资料库已保留。点击“开始持续聆听与回答”进行新测试。`);
      await refresh();
    } catch (e) {
      setPhase('清理未完成 · 已停止自动回答');
      throw archivePath ? new Error(`${String(e)}；完整备份已保留在废纸篓：${archivePath}`) : e;
    } finally { clearingHistory.current = false; draftRecovery.writer.resume(); }
  });
  const clearResidue = () => run(async()=>{
    if(clearingHistory.current || sessionOperation.current || recording.isRecording || requests.current.size>0 || historyRef.current.length || question || confirmedAnswer || status?.session_active)throw Error('请先结束当前会话，保留当前窗口内容');
    clearingHistory.current=true;sessionOperation.current=true;
    const wasSuspended=indexedDBService.recordingWritesSuspended();
    let archivePath='';
    indexedDBService.suspendRecordingWrites();
    try {
      const recovery=await indexedDBService.snapshotInterviewHistory();
      const report=await invoke<{archive_path:string;recording_folders:number;database_meetings:number;staged_archives:number}>('interview_residue_clear',{recovery});
      archivePath=report.archive_path;
      await indexedDBService.removeArchivedInterviewHistory(recovery);
      setNotice(`上次残留已移入废纸篓：${report.recording_folders} 个记录目录、${report.database_meetings} 条会议、${report.staged_archives} 个清理备份及恢复缓存。已保存会话、个人资料和模型保留。`);
    }catch(e){throw archivePath?new Error(`${String(e)}；完整备份在废纸篓：${archivePath}`):e;}
    finally {
      if(!wasSuspended)indexedDBService.resumeRecordingWrites();
      clearingHistory.current=false;sessionOperation.current=false;setResidueRevision(n=>n+1);
    }
  });
  const toggleMicrophone = (enabled: boolean) => run(async () => {
    microphoneVersion.current++;
    microphoneChanging.current = true;
    microphoneGate.current.close();
    try {
      if (recording.isRecording) {
        applyMicrophoneState(await invoke<MicrophoneState>('interview_microphone_set', { enabled }));
      } else { setRecordMicrophone(enabled); }
      try {localStorage.setItem('tingxian-microphone',enabled?'on':'off');} catch {}
      setMicLevel(0);
    } catch (e) {
      const levels = await invoke<{microphone_state:MicrophoneState}>('interview_audio_level').catch(() => null);
      if (levels) applyMicrophoneState(levels.microphone_state);
      throw e;
    } finally { microphoneChanging.current = false; }
  });
  const toggleFile = (file: File, on: boolean) => setSelected(s => {
    const next = { ...s }; if (on) next[file.path] = { path: file.path, kind: isStructuredPath(file.path) ? 'technical' : /面试资料|面试准备|练习/.test(file.path) ? 'practice' : 'personal', allow_external: true }; else delete next[file.path]; return next;
  });

  const composer=<><LiveTranscription items={transcripts}/><QuestionComposer value={question} editingSelf={!!editingSelfId} busy={busy} cloudAllowed={cloudAllowed} onChange={setQuestion} onSubmit={()=>void submit(question)} onLocalSearch={localSearch} onCancelEdit={()=>{selfEditBase.current=null;setEditingSelfId(null);setQuestion('');setNotice('已取消修改，原记录保留。');}} /></>;
  const transcriptTools = (
    <details className="desk-transcripts"><summary>原始转写与本人回答记录</summary><div className="max-h-48 overflow-auto space-y-2 my-3">{transcripts.map(t => <button key={`${t.capture_session ?? 0}:${t.sequence_id}`} className="block text-left w-full p-2 rounded bg-slate-50" onClick={() => setQuestion(t.text)}>{t.timestamp} · {t.text}</button>)}</div><textarea aria-label="本轮实际回答" className="border rounded w-full h-20 p-2" value={confirmedAnswer} onChange={e => setConfirmedAnswer(e.target.value)} placeholder="可选：记录最近一轮自己实际说过的回答，供后续追问使用。" /><Button variant="outline" disabled={!confirmedAnswer.trim() || !history.length} onClick={() => { setHistory(h => { const last = h.map(t => t.role !== 'self' && t.status === 'done').lastIndexOf(true); return h.map((t,i) => i === last ? { ...t, confirmed_answer: confirmedAnswer } : t); }); setConfirmedAnswer(''); }}>确认本人实际回答</Button></details>
  );
  const panelOpen = settings || savedSessions !== null;
  const closeSettings = () => { setSettings(false); settingsTrigger.current?.focus(); };
  const sendCommand=useSessionCommands(async(command)=>{
    if(busy||sessionOperation.current)return;
    if(command==='audio'){setSavedSessions(null);setSettingsTab('音频');setSettings(true);return;}
    if(command==='history'){setSettings(false);await listSessions();return;}
    if(command==='cancel'){await cancel().catch(error);return;}
    if(command==='clear'){await clearHistory();return;}
    if(command==='toggle')command=recording.isRecording?'stop':'start';
    if(command==='start'){
      if(recording.isRecording)return;
      if(!audioAllowed){setNotice('请先勾选「已获允许使用转写」，再开始采集。');return;}
      if(!status){setNotice('正在读取本机设置，请稍后重试。');return;}
      if(status.cloud_asr_mode==='aliyun'&&!asrCloudAllowed){setNotice('请勾选本次云转写授权，或在音频设置切回本地。');return;}
      await run(async()=>{
          audioHealth.current.prepareStart(); syncHealth();
          meeting.reset(); utterances.current.reset(); setQueued(0); setFrozen(false); setAutomatic(cloudAllowed && !!status?.key_configured); resumeAutomatic.current=false;
          captureSession.current++;
          acceptTranscripts.current = true;
          live.current.automatic = cloudAllowed && !!status?.key_configured;
          microphoneChanging.current = true;
          microphoneVersion.current++;
          try {
            applyMicrophoneState(await invoke<MicrophoneState>('interview_audio_start', { recordMicrophone,cloudAsrAllowed:asrCloudAllowed }));
            setPhase(listeningPhase(live.current.automatic,status.cloud_asr_mode==='aliyun',addressedOnly(meeting.current.current.mode)));
          } catch (e) { setAutomatic(false); live.current.automatic = false; audioHealth.current.close(); syncHealth(); throw e; }
          finally { microphoneChanging.current = false; }
        });
    }else if(command==='stop'&&recording.isRecording){
      await run(async()=>{
          live.current.automatic = false; setAutomatic(false); await cancel(); await stopInterviewAudio();
          microphoneGate.current.close(); audioHealth.current.close(); syncHealth(); setHistory(h => h.map(t => t.status === 'listening' ? {...t,status:'recorded'} : t)); setMicLevel(0); setLevel(0); setPhase('采集已停止 · 可直接重新开始');
        });
    }else if(recording.isRecording&&(command==='pause'&&!recording.isPaused||command==='resume'&&recording.isPaused)){
      await run(async()=>{
          if (recording.isPaused) {
            try { await invoke('resume_recording'); }
            catch (e) { setPhase('恢复未确认 · 自动回答保持关闭'); throw e; }
            const enable = resumeAutomatic.current && live.current.cloudAllowed && !!status?.key_configured && !audioHealth.current.paused;
            live.current.automatic = enable; setAutomatic(enable);
            setPhase(listeningPhase(enable,status?.cloud_asr_mode==='aliyun',addressedOnly(meeting.current.current.mode)));
          } else {
            resumeAutomatic.current=live.current.automatic; live.current.automatic = false; setAutomatic(false);
            try { await cancel(); await invoke('pause_recording'); setPhase('采集已暂停'); }
            catch (e) { setPhase('暂停未确认 · 自动回答保持关闭'); throw e; }
          }
        });
    }
  });
  return <main aria-label="面试辅助工作区" className={`interview-desk${split ? ' desk-split' : ''}`}>
    <aside aria-label="面试操作区" className="desk-rail" data-expanded={compactControls}>
      <header className="desk-brand">
        <img alt="" src="/tingxian.png" width="34" height="34"/><h1>听弦</h1>
        <div className="desk-state" data-listening={recording.isRecording && !recording.isPaused}>
          <span aria-hidden="true" className="desk-state-dot"/><span role="status" title={phase} aria-label={phase}>{phase.split(' · ')[0]}</span>
        </div>
        <button className="tx-compact-toggle" type="button" aria-expanded={compactControls} onClick={()=>setCompactControls(v=>!v)}>{compactControls?'收起操作':'展开操作'}</button>
      </header>
      <section aria-label="采集与回答操作" className="desk-controls">
      <Button type="button" variant="outline" className="w-full justify-start border-[#C5C7CA] bg-white" disabled={windowChanging} onClick={async () => {
        setWindowChanging(true);
        try { setSplit(await invoke<boolean>('interview_window_mode', { enabled: !split })); }
        catch (e) { error(e); }
        finally { setWindowChanging(false); }
      }}>
        {split ? <AppWindow aria-hidden="true"/> : <Columns2 aria-hidden="true"/>}
        {split ? '合并窗口' : '拆分窗口'}
      </Button>
      <MeetingControls setup={meeting.setup} change={meeting.change}/>
      <div className="desk-permissions"><label><input type="checkbox" checked={audioAllowed} onChange={e => setAudioAllowed(e.target.checked)} /> 已获允许使用转写</label>{status?.cloud_asr_mode==='aliyun'&&<label><input type="checkbox" disabled={busy || recording.isRecording} checked={asrCloudAllowed} onChange={e=>setAsrCloudAllowed(e.target.checked)}/> 允许将本次声音和术语发送至百炼实时转写</label>}<label><input type="checkbox" checked={cloudAllowed} onChange={e => { setCloudAllowed(e.target.checked); if (!e.target.checked) { setAutomatic(false); void cancel().catch(error); } }} /> 允许向已启用的 AI 发送对方发言、近期对话和选定资料</label></div>

        <label className="desk-check" title="本软件的麦克风独立于飞书静音">
          <input type="checkbox" checked={recordMicrophone} disabled={busy || recording.isStopping} onChange={e=>void toggleMicrophone(e.target.checked)}/> 记录我的声音
        </label>
        <label className="desk-check">
          <input type="checkbox" checked={automatic} disabled={!cloudAllowed || !recording.isActive || healthView.paused}
            onChange={e=>{const enable=e.target.checked&&!audioHealth.current.paused;live.current.automatic=enable;setAutomatic(enable);}}/> {addressedOnly(meeting.setup.mode)?'点到我时自动回答':'对方说完后自动回答'}
        </label>
        <div className="desk-transport">
        <Button hidden={recording.isRecording} disabled={!audioAllowed || recording.isRecording || busy} onClick={()=>sendCommand('start')}><Play className="h-4 w-4 mr-1" />开始持续聆听</Button>
        <Button variant="outline" hidden={!recording.isRecording} disabled={!recording.isRecording || busy} onClick={()=>sendCommand(recording.isPaused?'resume':'pause')}><Pause className="h-4 w-4 mr-1" />{recording.isPaused ? '恢复' : '暂停'}</Button>
        <Button variant="outline" hidden={!recording.isRecording} disabled={!recording.isRecording || busy} onClick={()=>sendCommand('stop')}><Square className="h-4 w-4 mr-1" />停止采集</Button>

        </div>
      {healthView.paused && <div role="alert" className="rounded-lg border border-amber-300 bg-amber-50 p-3 text-sm space-y-2"><p>{healthView.message}</p><Button size="sm" variant="outline" disabled={busy || !recording.isActive || !healthView.canResume} onClick={() => {
        if (!recording.isActive || !audioHealth.current.resume()) return;
        utterances.current.clear(); setQueued(0); syncHealth();
        const enable = healthResumeAutomatic.current && live.current.cloudAllowed && !!status?.key_configured;
        live.current.automatic = enable; setAutomatic(enable); setPhase(listeningPhase(enable,status?.cloud_asr_mode==='aliyun',addressedOnly(meeting.current.current.mode)));
      }}>{healthResumeAutomatic.current && cloudAllowed && status?.key_configured ? '确认声音恢复，继续自动回答' : '确认声音恢复，继续转写'}</Button><p className="text-xs">请确认音量和字幕已恢复。故障前排队及故障期间的发言不会自动补发；可编辑后重问。仍异常时点击“停止采集”再开始。</p></div>}
      {healthView.advisory && <p role="status" className="text-xs text-amber-800">{healthView.advisory}</p>}
      <div className="grid gap-2" hidden={!recording.isActive && !generating && !queued && !frozen}><Button size="sm" variant={frozen ? 'default' : 'outline'} onClick={() => { const next = !frozen; setFrozen(next); if (next) { void cancel().catch(error); setPhase('我在回答 · 已冻结'); } }}>{frozen ? '解除冻结' : '我在回答 / 冻结'}</Button><Button size="sm" variant="outline" disabled={!generating && !queued} onClick={()=>sendCommand('cancel')}>取消当前与队列</Button></div>
        <Button className="w-full" variant="outline" title={status?.vision_ready ? `向 ${status.vision_model} 发送所选整屏和近期对话` : "打开视觉模型配置"} disabled={busy||!!editingSelfId} onClick={()=>void submit(question.trim()||'请识别屏幕中的完整题目，并结合本次双方对话提供答案。',false,undefined,true)}><Camera size={16} aria-hidden="true"/>截屏并回答</Button>

      </section>
      <CallNotice text={split ? meeting.notice : null}/><div className="desk-tools">
        <details className="desk-fold"><summary>屏幕与显示</summary><div>
        <label className="block">对话字号<ChoiceMenu aria-label="对话字号" className="ml-2" value={preferences.fontSize} onValueChange={nextValue =>changePreferences({...preferences,fontSize:nextValue as PresentationPreferences['fontSize']})}><ChoiceOption value="small">小</ChoiceOption><ChoiceOption value="medium">中</ChoiceOption><ChoiceOption value="large">大</ChoiceOption></ChoiceMenu></label>
        <label className="block"><input type="checkbox" checked={preferences.showSelf} onChange={e=>changePreferences({...preferences,showSelf:e.target.checked})} /> 展示我的发言</label>
        <label className="block">截屏范围<ChoiceMenu aria-label="截屏范围" className="w-full" value={screenId??''} onValueChange={nextValue =>setScreenId(nextValue?Number(nextValue):null)}><ChoiceOption value="">当前主屏幕（完整屏幕）</ChoiceOption>{screenDisplays.map((d,i)=><ChoiceOption key={d.id} value={d.id}>屏幕 {i+1}{d.primary?' · 主屏':''} · {d.width}×{d.height}</ChoiceOption>)}</ChoiceMenu></label>
        <Button size="sm" variant="outline" disabled={busy} onClick={async()=>{try{const screens=await invoke<ScreenDisplay[]>('interview_screenshot_displays');setScreenDisplays(screens);if(screenId&&!screens.some(d=>d.id===screenId))setScreenId(null);}catch(e){error(e);}}}>刷新屏幕列表</Button>

          <small>截屏会将所选整屏和近期对话发送至视觉模型。</small>
        </div></details>
        {history.length>0&&<details className="desk-fold"><summary>本次会话</summary><div><Button variant="outline" size="sm" disabled={busy || !history.length} onClick={()=>run(saveSessionSnapshot)}>保存当前会话到本机</Button><Button variant="outline" size="sm" onClick={exportSession}>导出本次记录</Button><Button variant="outline" size="sm" disabled={busy || !history.length} onClick={end}>结束会话并保留历史</Button></div></details>}
        <details className="desk-fold"><summary>连接与用量</summary><div>
          <div className="desk-levels">
            <label>系统<meter min="0" max="1" value={Math.min(1,level)} /></label>
            <label>麦克风<meter min="0" max="1" value={Math.min(1,micLevel)} /></label>
          </div>
    <div className="flex flex-wrap gap-x-5 gap-y-2 text-xs text-slate-500">
      <span>可用回答通道 {status?.answer_capacity??'…'} / 3</span>
      <span>索引 v{status?.index_version || '0'}{status?.session_active ? ' · 会话锁定' : ''}</span>
      <span>{timing || '等待本次请求耗时'}</span><span>最近用量 {usage}</span>
      <span title="仅累计服务实际返回的 total_tokens；重试、取消或中断没有 usage 的部分保留未知，不能当作账单。连接自检单独计量。">{usageLabel(usageLedger.current.snapshot())}</span>
    </div>

        </div></details>
        <nav className="tx-rail-nav" aria-label="工作台导航">
          <button type="button" aria-current={savedSessions!==null?'page':undefined} onClick={()=>sendCommand('history')}><HistoryIcon/>会议记录<ChevronRight/></button>
          <button type="button" aria-current={settings&&settingsTab==='音频'?'page':undefined} onClick={()=>sendCommand('audio')}><AudioLines/>音频设置<ChevronRight/></button>
          <button ref={settingsTrigger} type="button" aria-current={settings&&settingsTab!=='音频'?'page':undefined} onClick={()=>{setSettingsTab('资料库');setSettings(true);setSavedSessions(null);}}><Settings2/>资料与模型<ChevronRight/></button>
        </nav>
      </div>
      <footer className="desk-foot">
        <div className="desk-count">本次调用 {calls} / {limit}{queued>0 && <span> · 待处理 {queued} 段</span>}</div>
    {limit-calls <= REQUEST_LIMIT_WARNING && <p role="status" className="text-xs text-amber-800">
      {calls < limit ? `本轮还可请求 ${limit-calls} 次建议回答，可提前点击“继续 20 次”增加额度。` : '已达到本轮请求上限；正在生成的回答会继续完成，采集仍可继续。点击“继续 20 次”后可重新开启自动回答。'}
    </p>}

        {limit-calls<=REQUEST_LIMIT_WARNING && <Button variant="outline" size="sm" disabled={busy} onClick={()=>setLimit(l=>l+20)}>继续 20 次</Button>}
    {draftRecovery.drafts.length>0 && <Button variant="outline" size="sm" onClick={()=>void listSessions()}>恢复草稿（{draftRecovery.drafts.length}）</Button>}
    {draftRecovery.error && <div role="alert" className="text-xs text-amber-800"><p>恢复草稿未保存：{draftRecovery.error}</p><Button size="sm" variant="outline" onClick={()=>void draftRecovery.writer.retry().catch(error)}>重试保存草稿</Button></div>}
    <ResidueNotice blocked={busy || recording.isRecording || generating || history.length>0 || !!question || !!confirmedAnswer || !!status?.session_active} revision={residueRevision} onClear={clearResidue} />

        {!recording.isRecording && !generating && !busy && !split && <Link className="tx-switch-workspace" href="/interview">通用对话助手 <span aria-hidden="true">↗</span></Link>}
      </footer>
      {split && <div className="desk-split-composer">
        {(notice||answerWarning) && <div role="status">{notice||answerWarning}</div>}
        {composer}
        {phase.startsWith('仅本地') && <div className="text-xs space-y-2"><ConflictNotice conflicts={conflicts} onReview={()=>{setSettingsTab('事实与术语');setSettings(true);}} />{sources.map(source => <details key={source.source_id}><summary>{source.path} · {source.location} · {sourceStatusLabel(source.metadata)}</summary><p className="whitespace-pre-wrap">{source.text}</p></details>)}</div>}
        {(transcripts.length>0 || history.length>0) && transcriptTools}
      </div>}
    </aside>
    <div aria-label="识别与对话区" className={`desk-stage${split && panelOpen ? ' desk-stage-overlay' : ''}`} hidden={split&&!panelOpen}>
    <CallNotice text={meeting.notice}/>
    {notice && <div role="alert" className="rounded-lg border border-amber-200 bg-amber-50 px-4 py-3 text-sm break-words">{notice}</div>}
    {generating && answerWarning && <div role="status" className="rounded-lg border border-sky-200 bg-sky-50 px-4 py-3 text-sm break-words">{answerWarning}</div>}
    {importJob && <section role="status" aria-live="polite" className="rounded-lg border bg-slate-50 p-3 text-sm flex items-center justify-between gap-3">
      <div><p>{importJob.cancelPending ? '正在取消导入，保留原资料库…' : importPhaseNames[importJob.phase] || importJob.phase} · {importJob.completed} / {importJob.total}</p>{importJob.path && <p className="text-xs text-slate-500 break-all">{importJob.path}</p>}</div>
      <Button variant="outline" disabled={!importJob.ready || importJob.cancelPending || importJob.phase === 'committing'} onClick={cancelImport}>取消导入</Button>
    </section>}
    {settings && <section className="desk-panel" aria-label="资料与设置" onKeyDown={e=>{if(e.key==='Escape' && !e.nativeEvent.isComposing && !e.defaultPrevented && !(e.target as Element).closest('[role=listbox]'))closeSettings();}}>
      <header className="desk-panel-heading"><h2>设置</h2><Button autoFocus variant="ghost" size="sm" onClick={closeSettings}>返回对话</Button></header>
      <nav aria-label="设置分类" className="desk-tabs">{(['音频','资料库','事实与术语','模型接口'] as const).map(tab=><button key={tab} type="button" aria-pressed={settingsTab===tab} onClick={()=>setSettingsTab(tab)}>{tab}</button>)}</nav>
      <div className="desk-panel-body">
      {settingsTab==='音频'&&<AudioSettings locked={busy || recording.isRecording} microphone={recordMicrophone} onMicrophone={enabled=>void toggleMicrophone(enabled)} onConfigurationChanged={()=>{setAsrCloudAllowed(false);void refresh();}}/>}
      <div hidden={settingsTab!=='资料库'} className="space-y-4">

      <div className="flex flex-wrap items-center justify-between gap-3 text-xs">
        <p className="text-slate-500">已索引 {status?.index_documents ?? '…'} 份资料 · {status?.index_chunks ?? '…'} 个片段。</p>
        <Button variant="outline" size="sm" disabled={busy || !status || status.session_active} onClick={clearIndex}>移入废纸篓并清空索引</Button>
      </div>
      <p className="text-xs text-slate-500">{status ? `${status.architecture} · macOS ${status.system_version} · ${(status.memory_bytes / 1024 ** 3).toFixed(0)} GB · Whisper：${status.whisper_model || '未加载'}` : '等待桌面自检'}</p>
      <BackgroundLibrary locked={busy || !status || !!status.session_active || recording.isRecording || generating} active={settingsTab==='资料库'} revision={status?.index_version} names={meeting.setup.names} applyNames={meeting.applyNames} onBusy={setBusy} onApplied={refresh} onModels={()=>{setOpenVision(false);setOpenBackground(true);setSettingsTab('模型接口');}}/>
      <details><summary>手动选择文件与字段</summary><div className="space-y-3 pt-2">
      <div className="flex gap-2"><Input aria-label="资料根目录" disabled={busy || status?.session_active} placeholder="选择已有个人资料目录" value={root} onChange={e => { setRoot(e.target.value); setFiles([]); setSelected({}); }} /><Button variant="outline" disabled={busy || status?.session_active} onClick={() => run(async () => { const p = await invoke<string | null>('profile_choose'); if (p) { setRoot(p); setFiles([]); setSelected({}); } })}><FolderOpen className="h-4 w-4" aria-label="选择资料目录" /></Button><Button disabled={busy || !root || status?.session_active} onClick={() => run(async () => { setFiles(await invoke<File[]>('profile_scan', { root })); setSelected({}); })}>扫描预览</Button></div>
      {files.length > 0 && <><Input aria-label="筛选文件" value={filter} onChange={e => setFilter(e.target.value)} placeholder="筛选文件名，例如：背景、SELECTTEST" /><div className={`${Object.values(selected).some(s => isStructuredPath(s.path)) ? 'max-h-[36rem]' : 'max-h-64'} overflow-auto divide-y rounded border`}>
        {files.filter(f => f.path.toLowerCase().includes(filter.toLowerCase())).slice(0,300).map(file => <div key={file.path} className="p-2 text-xs flex flex-wrap items-center gap-2">
          <input aria-label={`纳入 ${file.path}`} type="checkbox" disabled={busy || !file.eligible || status?.session_active} checked={!!selected[file.path]} onChange={e => toggleFile(file, e.target.checked)} /><span className="flex-1 min-w-40 break-all">{file.path}</span><span className="text-slate-400">{(file.bytes/1024).toFixed(0)} KB · {file.reason}</span>
          {selected[file.path] && <><ChoiceMenu disabled={busy || status?.session_active} aria-label="资料类型" value={selected[file.path].kind} onValueChange={nextValue => setSelected(s => ({ ...s, [file.path]: { ...s[file.path], kind: nextValue } }))}>{Object.entries(kindNames).map(([v,n]) => <ChoiceOption key={v} value={v}>{n}</ChoiceOption>)}</ChoiceMenu><label><input type="checkbox" disabled={busy || status?.session_active} checked={selected[file.path].allow_external} onChange={e => setSelected(s => ({ ...s, [file.path]: { ...s[file.path], allow_external: e.target.checked } }))} /> 允许相关片段外发</label></>}
          {selected[file.path] && isStructuredPath(file.path) && <StructuredFields key={JSON.stringify([root,file.path])} root={root} path={file.path} value={selected[file.path].structured} locked={busy || !!status?.session_active} onBusyChange={setBusy} onChange={structured => setSelected(current => current[file.path] ? { ...current, [file.path]: { ...current[file.path], structured } } : current)} />}
        </div>)}
      </div><p className="text-xs text-slate-500">扫描 {files.length} 项，当前选择 {Object.keys(selected).length} 项。列表最多展示 300 项，可输入名称筛选。重新导入会移除未选择项目的活动索引。{incompleteStructured && '所选 JSON/YAML 尚未选择字段，暂不能导入。'}</p><Button disabled={busy || status?.session_active || incompleteStructured} onClick={startImport}>导入 / 更新所选资料</Button></>}
      </div></details>
      <div className="space-y-2">
        <div className="flex items-center justify-between gap-3"><h3 className="text-sm font-medium">资料核对与时效</h3><Button variant="outline" size="sm" disabled={busy} onClick={()=>run(refreshDocuments)}>刷新已导入资料</Button></div>
        <p className="text-xs text-slate-500 break-all">当前索引目录：{status?.root || '尚未导入'}。内容变化后旧确认失效；日期由本人设置，不以文件修改时间代替。会话期间仅可查看标记。</p>
        {(status?.documents || []).map(document=><DocumentReview key={JSON.stringify([document.root,document.path,document.hash,document.metadata])} document={document} locked={busy || !!status?.session_active} onSave={update=>run(async()=>{await invoke('profile_document_update',{update});await refreshDocuments();setNotice('资料核对与日期已保存；原文件未修改。');})} />)}
      </div>
      {report && <pre className="max-h-48 overflow-auto bg-slate-50 p-3 rounded text-xs whitespace-pre-wrap">{report}</pre>}
      </div>
      <div hidden={settingsTab!=='事实与术语'} className="space-y-4">
      {status && <FactReview card={status.fact_card || legacyFactCard(status.facts)} candidates={sources} locked={busy || status.session_active} onBusyChange={setBusy} onRefresh={async()=>{const next=await invoke<FactCard>('profile_facts_current');setStatus(current=>current?{...current,facts:next.text,fact_card:next}:current);return next;}} onSave={async update=>{
        const next=await invoke<FactCard>('profile_facts_correct',{update});
        setStatus(current=>current?{...current,facts:next.text,fact_card:next}:current);setNotice(`事实卡 v${next.version} 已确认保存，旧值与来源保留。`);return next;
      }} />}
      <label className="block text-sm">术语别名（每行一组，英文逗号分隔）<textarea className="w-full border rounded p-2 mt-1 h-28 text-sm" value={aliases} onChange={e => setAliases(e.target.value)} /></label>
      <Button disabled={busy || status?.session_active} onClick={()=>run(async()=>{await invoke('interview_config',{aliases,key:null});await refresh();})}>保存术语别名</Button>
      </div>
      <div hidden={settingsTab!=='模型接口'}><ProviderSettings openVision={openVision} openBackground={openBackground} locked={busy || !!status?.session_active || recording.isRecording || generating} onBusyChange={setBusy} onSaved={refresh} /></div>
      </div>
    </section>}
    {savedSessions !== null && <section className="desk-panel" aria-label="会议记录">
      <header className="desk-panel-heading"><h2>会议记录</h2><Button variant="ghost" size="sm" onClick={()=>setSavedSessions(null)}>返回对话</Button></header>
      <div className="desk-panel-body"><div className="tx-history">
      <section><header className="tx-section-head"><h3>已保存会话</h3></header>{(history.length>0 || question || confirmedAnswer) && <label className="block"><input type="checkbox" disabled={busy || recording.isRecording || generating} checked={allowReplaceSession} onChange={e=>setAllowReplaceSession(e.target.checked)} /> 我已保存需要的内容，允许替换当前显示（未保存内容会丢失）</label>}{!savedSessions.length && <p className="tx-empty-line">暂无已保存会话</p>}{savedSessions.map(s=><div key={s.id} className="tx-history-row"><span>{new Date(s.saved_at).toLocaleString('zh-CN')} · {s.turns} 条发言 · {s.calls} 次调用</span><Button size="sm" variant="outline" disabled={busy || recording.isRecording || generating || (!!(history.length || question || confirmedAnswer) && !allowReplaceSession)} onClick={()=>restoreSession(s.id)}>恢复这份会话</Button><SavedSessionPreview id={s.id}/></div>)}{savedBefore!==null && <Button variant="outline" disabled={busy} onClick={()=>listSessions(true)}>更早的保存记录</Button>}</section>
      <PreviousMeetings initialId={initialMeetingId}/>
      <RecoveryDrafts drafts={draftRecovery.drafts} disabled={busy || recording.isRecording || generating || (!!(history.length || question || confirmedAnswer) && !allowReplaceSession)} onRestore={id=>void restoreSession(id)}/>
      <div className="tx-session-actions">{history.length>0 && <div><Button variant="outline" size="sm" disabled={busy || !history.length} onClick={()=>run(saveSessionSnapshot)}>保存当前会话到本机</Button><Button variant="outline" size="sm" onClick={exportSession}>导出本次记录</Button><Button variant="outline" size="sm" disabled={busy || !history.length} onClick={end}>结束会话并保留历史</Button></div>}<details><summary>清理历史</summary><p>已保存会话将一并移入废纸篓。</p><Button variant="outline" size="sm" disabled={busy} onClick={()=>sendCommand('clear')}>清空历史，重新开始</Button></details></div>
      </div></div></section>}
    {!split && <div className="desk-reading" hidden={panelOpen}>
      <AnswerConversation key={feedGeneration} history={history} preferences={preferences} onEdit={editTurn} />
      <div className="desk-compose-dock">{composer}
        {phase.startsWith('仅本地') && <div className="text-xs space-y-2"><ConflictNotice conflicts={conflicts} onReview={()=>{setSettingsTab('事实与术语');setSettings(true);}} />{sources.map(source => <details key={source.source_id}><summary>{source.path} · {source.location} · {sourceStatusLabel(source.metadata)}</summary><p className="whitespace-pre-wrap">{source.text}</p></details>)}</div>}
      </div>
      {(transcripts.length>0 || history.length>0) && transcriptTools}
    </div>}

    </div>
  </main>;
}
