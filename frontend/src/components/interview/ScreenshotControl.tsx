'use client';
import {forwardRef,useEffect,useImperativeHandle,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {Camera} from 'lucide-react';
import {Button} from '@/components/ui/button';
import {ChoiceMenu,ChoiceOption} from '@/components/ui/choice-menu';
import {Dialog,DialogContent,DialogTitle,DialogDescription} from '@/components/ui/dialog';

type Screen={id:number;width:number;height:number;primary:boolean};
export type ScreenshotContext={questionId:string;question:string;history:{question:string;confirmed_answer:string|null}[];dialogue:{source:string;text:string}[];meetingMode:string};
type Image={preview_id:string;data_url:string;display_id:number;topology:string;expires_in_seconds:number;requires_confirmation:boolean};
export type PreparedScreenshot={context:ScreenshotContext;image:Image};
export type ScreenshotHandle={invalidate:(reason?:string)=>void};
type Props={disabled:boolean;canSend:boolean;contextKey:string;makeContext:()=>ScreenshotContext;onSend:(capture:PreparedScreenshot)=>Promise<boolean>;onNotice:(message:string)=>void};
type View={open:boolean;screens:Screen[];selected:number|null;preview:PreparedScreenshot|null;busy:null|'loading'|'capturing'|'sending';error:string};
const initial:View={open:false,screens:[],selected:null,preview:null,busy:null,error:''};
const signature=(screens:Screen[])=>JSON.stringify([...screens].sort((a,b)=>a.id-b.id));

export const ScreenshotControl=forwardRef<ScreenshotHandle,Props>(function ScreenshotControl(props,ref){
  const [view,setView]=useState<View>(initial),[quick,setQuick]=useState(false);
  const current=useRef(view),latest=useRef(props),version=useRef(0),pending=useRef<ScreenshotContext|null>(null),confirmed=useRef<string|null>(null);
  const quickRef=useRef(false),expiry=useRef<ReturnType<typeof setTimeout>|null>(null);
  const cleanup=useRef<Promise<void>>(Promise.resolve());
  latest.current=props;
  const change=(patch:Partial<View>)=>{current.current={...current.current,...patch};setView(current.current);};
  const discard=(context:ScreenshotContext)=>{cleanup.current=cleanup.current.then(()=>invoke<void>('interview_screenshot_preview_discard',{questionId:context.questionId})).catch(e=>latest.current.onNotice(String(e)));return cleanup.current;};
  const invalidate=(reason?:string)=>{
    if(!pending.current&&!current.current.open&&!current.current.busy)return;
    version.current++;if(expiry.current)clearTimeout(expiry.current);expiry.current=null;
    const context=pending.current;pending.current=null;if(context)discard(context);
    change({open:false,preview:null,busy:null,error:''});if(context&&reason)latest.current.onNotice(reason);
  };
  useImperativeHandle(ref,()=>({invalidate}));
  useEffect(()=>{
    try{quickRef.current=localStorage.getItem('tingxian-screenshot-quick')==='on';setQuick(quickRef.current);
      const stored=localStorage.getItem('tingxian-screenshot-display');if(stored&&/^\d+$/.test(stored))change({selected:Number(stored)});
    }catch{}
    return()=>{version.current++;if(expiry.current)clearTimeout(expiry.current);if(pending.current)discard(pending.current);pending.current=null;};
    // Callbacks intentionally read latest props; preferences contain no images.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  },[]);
  useEffect(()=>{invalidate('问题或会话已变化，截图未发送，请重新截取。');},[props.contextKey]); // eslint-disable-line react-hooks/exhaustive-deps
  useEffect(()=>{if(!props.canSend&&current.current.busy==='sending')invalidate('发送许可已变化。');},[props.canSend]); // eslint-disable-line react-hooks/exhaustive-deps

  const send=async(capture:PreparedScreenshot,stamp:string,token:number)=>{
    if(token!==version.current)return;
    if(!latest.current.canSend){change({busy:null,error:'请先允许 AI 发送并配置视觉模型。'});return;}
    change({busy:'sending',error:''});
    try{
      const accepted=await latest.current.onSend(capture);
      if(token!==version.current)return;
      if(!accepted){change({busy:null,error:'未发送，请检查用量上限或等待回答通道空闲。'});return;}
      confirmed.current=stamp;pending.current=null;
      if(expiry.current)clearTimeout(expiry.current);expiry.current=null;
      version.current++;change({open:false,preview:null,busy:null,error:''});
    }catch(e){if(token===version.current){confirmed.current=null;discard(capture.context);pending.current=null;change({preview:null,busy:null,error:String(e)});}}
  };
  const capture=async(id:number,screens:Screen[],fast:boolean,token:number)=>{
    if(token!==version.current)return;
    if(expiry.current)clearTimeout(expiry.current);expiry.current=null;
    if(pending.current){discard(pending.current);pending.current=null;}
    await cleanup.current;if(token!==version.current)return;
    const context=latest.current.makeContext();pending.current=context;
    change({open:true,preview:null,busy:'capturing',error:''});
    try{
      const image=await invoke<Image>('interview_screenshot_preview_create',{context,displayId:id});
      if(token!==version.current){discard(context);return;}
      const prepared={context,image};change({preview:prepared,busy:null});
      expiry.current=setTimeout(()=>invalidate('截图已过期，请重新截取。'),Math.min(300,image.expires_in_seconds)*1000);
      if(fast&&image.requires_confirmation===false)await send(prepared,signature(screens),token);
    }catch(e){if(token===version.current){discard(context);pending.current=null;confirmed.current=null;change({busy:null,selected:null,preview:null,error:String(e)});}}
  };
  const open=async(forcePreview=false)=>{
    if(latest.current.disabled||current.current.busy)return;
    const token=++version.current;change({open:true,busy:'loading',error:'',preview:null});
    try{
      const screens=await invoke<Screen[]>('interview_screenshot_displays');if(token!==version.current)return;
      const selected=screens.some(s=>s.id===current.current.selected)?current.current.selected:screens.length===1?screens[0].id:null;
      const unchanged=confirmed.current===signature(screens);if(!unchanged)confirmed.current=null;
      change({screens,selected,busy:null,error:screens.length?'':'没有可用屏幕，请连接后重试。'});
      if(!forcePreview&&quickRef.current&&unchanged&&selected!==null&&latest.current.canSend)await capture(selected,screens,true,token);
    }catch(e){if(token===version.current){confirmed.current=null;change({busy:null,error:String(e)});}}
  };
  return <>
    <div className="flex gap-2"><Button className="flex-1" variant="outline" disabled={props.disabled||!!view.busy} onClick={()=>void open()}><Camera size={16} aria-hidden="true"/>截屏答题</Button>{quick&&<Button variant="outline" aria-label="预览截图与调整快速发送" disabled={props.disabled||!!view.busy} onClick={()=>void open(true)}>预览</Button>}</div>
    <Dialog open={view.open} onOpenChange={open=>{if(!open&&view.busy!=='sending')invalidate();}}>
      <DialogContent className="max-w-4xl max-h-[90dvh] overflow-y-auto bg-white text-[#17181A]">
        <DialogTitle>截屏答题</DialogTitle>
        <DialogDescription>仅本次发送；图片不写入会话记录。</DialogDescription>
        <label className="flex flex-wrap items-center gap-3">屏幕<ChoiceMenu aria-label="截图屏幕" disabled={!!view.busy} value={view.selected??''} onValueChange={value=>{
          version.current++;if(expiry.current)clearTimeout(expiry.current);expiry.current=null;if(pending.current)discard(pending.current);pending.current=null;confirmed.current=null;
          const id=value===''?null:Number(value);change({selected:id,preview:null,error:''});try{if(id===null)localStorage.removeItem('tingxian-screenshot-display');else localStorage.setItem('tingxian-screenshot-display',String(id));}catch{}
        }}><ChoiceOption value="">选择屏幕</ChoiceOption>{view.screens.map((s,i)=><ChoiceOption key={s.id} value={s.id}>屏幕 {i+1}{s.primary?' · 主屏':''} · {s.width}×{s.height}</ChoiceOption>)}</ChoiceMenu></label>
        {view.preview?<figure className="m-0 rounded-xl border border-[#C5C7CA] bg-[#F4F5F6] p-2"><img className="max-h-[52dvh] w-full object-contain" src={view.preview.image.data_url} alt="本次将发送的完整屏幕截图"/><figcaption className="mt-2 break-words text-sm">{view.preview.context.question}</figcaption></figure>:<div className="grid min-h-40 place-items-center rounded-xl border border-dashed border-[#C5C7CA] text-sm">{view.busy==='capturing'?'正在截屏…':view.busy==='loading'?'读取屏幕…':'选择题目所在屏幕后截取预览'}</div>}
        {view.error&&<p role="alert" className="text-sm text-red-700">{view.error}</p>}
        {!props.canSend&&<p className="text-sm">发送前需允许 AI 发送并配置视觉模型。</p>}
        <div className="flex flex-wrap items-center justify-between gap-3">
          <label className="text-sm"><input type="checkbox" checked={quick} disabled={!!view.busy} onChange={e=>{quickRef.current=e.target.checked;setQuick(e.target.checked);try{localStorage.setItem('tingxian-screenshot-quick',e.target.checked?'on':'off');}catch{}}}/> 下次快速发送</label>
          <div className="flex gap-2"><Button variant="outline" disabled={view.busy==='sending'} onClick={()=>invalidate()}>取消</Button><Button variant="outline" disabled={!!view.busy||view.selected===null} onClick={()=>void capture(view.selected!,view.screens,false,++version.current)}>{view.preview?'重新截取':'截取预览'}</Button><Button disabled={!!view.busy||!view.preview||!props.canSend} onClick={()=>view.preview&&void send(view.preview,signature(view.screens),version.current)}>{view.busy==='sending'?'正在提交…':'发送并回答'}</Button></div>
        </div>
      </DialogContent>
    </Dialog>
  </>;
});
