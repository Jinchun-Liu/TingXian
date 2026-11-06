import type {TranscriptInput} from '@/lib/interview-scheduler';
export function LiveTranscription({items}:{items:TranscriptInput[]}){
  const caption=items.filter(t=>t.source==='System'&&t.is_partial&&!t.interrupted).at(-1);
  return caption?<p aria-label="实时字幕" className="shrink-0 max-h-16 overflow-y-auto px-6 py-2 text-sm text-slate-500">{caption.text}</p>:null;
}
