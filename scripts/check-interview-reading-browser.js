// Run against the exported display fixture created by build-interview-reading-fixture.cjs.
(async () => {
  const feed = document.querySelector('[aria-label="面试对话记录"]');
  const settle = () => new Promise(resolve => requestAnimationFrame(() => requestAnimationFrame(resolve)));
  const click = async label => {
    const button = [...document.querySelectorAll('button')].find(b => b.textContent === label);
    if (!button) throw Error('Missing button: ' + label);
    button.click(); await settle();
  };
  const check = (ok,msg) => { if (!ok) throw Error(msg); };
  const aligned = () => Math.abs(feed.querySelector('[data-latest-answer]').getBoundingClientRect().bottom - feed.getBoundingClientRect().bottom) <= 2;
  check(feed.scrollTop > 0 && aligned(), 'Initial answer is outside the viewport');
  let held=feed.scrollTop;
  await click('新增转写'); await click('更新转写');
  check(feed.scrollTop === held,'Transcript moved the viewport');
  await click('生成答复'); check(aligned(),'First answer text did not follow');
  feed.dispatchEvent(new WheelEvent('wheel',{deltaY:-180,bubbles:true}));
  feed.scrollTop-=180; await settle(); held=feed.scrollTop;
  await click('续写答复'); check(feed.scrollTop === held,'Stream interrupted manual reading');
  await click('返回最新答复'); check(aligned(),'Return missed the latest answer');
  await click('续写答复'); check(aligned(),'Return did not restore following');
  held=feed.scrollTop; await click('纠正问句'); check(feed.scrollTop === held,'Corrected question moved the viewport');
  await click('单题或列表'); await click('新增转写');
  check(feed.querySelector('article')?.querySelector('[data-latest-answer]'),'Unanswered turn replaced focus card');
  await click('清空记录'); check(!feed.isConnected || !feed.querySelector('[data-latest-answer]'),'Clear retained previous answer');
  return 'PASS';
})()
