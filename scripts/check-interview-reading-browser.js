// Run with browse eval against the isolated generated fixture, not the desktop app.
window.__readingCheck={status:'running',checks:0};
(async()=>{
  const results=[];
  const assert=(ok,message)=>{if(!ok)throw Error(message);results.push(message);window.__readingCheck.checks=results.length;};
  const settle=()=>new Promise(resolve=>setTimeout(()=>requestAnimationFrame(()=>requestAnimationFrame(resolve)),35));
  const feed=()=>document.querySelector('[aria-label="面试对话记录"]');
  const following=()=>document.querySelector('button[aria-pressed]')?.getAttribute('aria-pressed')==='true';
  const bottom=()=>feed().scrollHeight-feed().scrollTop-feed().clientHeight;
  const click=async label=>{const button=[...document.querySelectorAll('button')].find(e=>e.textContent===label);if(!button)throw Error('Missing control: '+label);button.click();await settle();};
  assert(!!feed() && feed().scrollHeight>feed().clientHeight,'actual browser has overflowing dialogue');
  assert(!following() && feed().scrollTop===0,'default reading mode starts at the beginning');
  feed().scrollTop=500;await settle();const original=feed().scrollTop;
  await click('新增对方问题');assert(feed().scrollTop===original,'new question preserves the reading position');
  await click('回到最新对话');assert(bottom()<=2 && !following(),'return to latest is a one-shot action');
  let held=feed().scrollTop;await click('流式追加一句');assert(feed().scrollTop===held && !following(),'one-shot navigation does not follow the next chunk');
  await click('跟随最新对话');await click('流式追加一句');assert(following() && bottom()<=2,'explicit following tracks streamed text');
  feed().scrollTop-=5;await settle();assert(!following(),'even scrolling slightly upward pauses following');
  held=feed().scrollTop;await click('流式追加一句');assert(feed().scrollTop===held,'streaming does not pull a reader back down');
  feed().scrollTop=feed().scrollHeight;await settle();assert(!following(),'manual scroll to bottom never opts back into following');
  held=feed().scrollTop;await click('流式追加一句');assert(feed().scrollTop===held,'new chunk preserves manually reached bottom position');
  await click('跟随最新对话');
  const summary=[...feed().querySelectorAll('summary')].at(-1);summary.focus();await settle();summary.click();await settle();
  assert(!following() && summary.parentElement.open,'keyboard focus and opening a source pause following');
  const sourceTop=summary.getBoundingClientRect().top;await click('流式追加一句');assert(Math.abs(summary.getBoundingClientRect().top-sourceTop)<=2 && !following(),'reading an expanded source keeps its visible position');
  feed().scrollTop=0;await settle();
  const paragraph=feed().querySelector('p');paragraph.dispatchEvent(new PointerEvent('pointerdown',{bubbles:true}));
  const range=document.createRange();range.setStart(paragraph.firstChild,0);range.setEnd(paragraph.firstChild,8);
  const selection=window.getSelection();selection.removeAllRanges();selection.addRange(range);const selected=selection.toString();
  await click('流式追加一句');assert(selection.toString()===selected && feed().scrollTop===0,'selected historical text survives new output without scrolling');
  selection.removeAllRanges();
  for(const [event,message] of [
    [new PointerEvent('pointerdown',{bubbles:true}),'pointer interaction pauses follow'],
    [new WheelEvent('wheel',{deltaY:-1,bubbles:true}),'upward wheel input pauses follow'],
    [new KeyboardEvent('keydown',{key:'PageUp',bubbles:true}),'upward keyboard navigation pauses follow'],
  ]){
    await click('跟随最新对话');feed().dispatchEvent(event);await settle();assert(!following(),message);
  }
  const composer=document.getElementById('composer');composer.focus();composer.value='正在输入自己的说明';
  await click('开始模拟输出');await new Promise(resolve=>setTimeout(resolve,500));await click('停止模拟输出');
  assert(document.activeElement===composer && composer.value==='正在输入自己的说明','continuous updates do not steal input focus or overwrite typing');
  await click('跟随最新对话');await click('清空演示记录');assert(!following() && feed().scrollTop===0,'clearing creates a fresh reading viewport');
  await click('跟随最新对话');await click('恢复合成历史');assert(!following() && feed().scrollTop===0,'restoring history starts in reading mode');
  return JSON.stringify({status:'passed',checks:results.length,results,scope:'Production ConversationFeed in isolated Chromium fixture; synthetic controls/data, no native capture or API',viewport:{width:innerWidth,height:innerHeight}});
})().then(result=>{window.__readingCheck=JSON.parse(result);}).catch(error=>{window.__readingCheck={...window.__readingCheck,status:'failed',error:String(error)};});
