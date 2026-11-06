export type ClockProbe={sent:number;worker:number;received:number};
/** Map worker performance.now() into the main monotonic clock without assuming
 * timeOrigin is identical. Lower-bound offset makes reported delays conservative
 * within the minimum RTT plus the timer rounding allowance, assuming stable
 * clock rates and <=2ms timer rounding over this <=60s browser preview. */
export function calibrateWorkerClock(probes:ClockProbe[]){
  if(probes.length!==3||probes.some(p=>![p.sent,p.worker,p.received].every(Number.isFinite)||p.received<p.sent))throw new Error("线程时钟校准数据无效。");
  const best=[...probes].sort((a,b)=>(a.received-a.sent)-(b.received-b.sent))[0];
  // performance.now() is quantized; a measured 0ms RTT is not perfect sync.
  // Reserve 2ms on each offset bound for timer rounding in this browser prototype.
  const measuredRttMs=best.received-best.sent,timerRoundingAllowanceMs=2;
  const uncertaintyMs=measuredRttMs+2*timerRoundingAllowanceMs;
  if(uncertaintyMs>10)throw new Error("线程通信延迟过大，无法校准；请重试。");
  return {method:"minimum_rtt_lower_bound",offsetMs:best.sent-best.worker-timerRoundingAllowanceMs,measuredRttMs,timerRoundingAllowanceMs,uncertaintyMs,probes};
}
