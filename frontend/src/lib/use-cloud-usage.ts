'use client';
import {useEffect,useRef,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {CloudUsage,CloudUsageSnapshot} from '@/lib/interview-cloud-usage';
// Both main-window routes share ongoing observations across client navigation.
// No credentials or audio enter this ledger. A new webview starts empty.
let windowLedger:CloudUsage|null=null;
export function useCloudUsage(){
  const ledger=useRef<CloudUsage|null>(null);if(!ledger.current){windowLedger??=new CloudUsage(invoke,()=>crypto.randomUUID());ledger.current=windowLedger;}
  const [value,setValue]=useState<CloudUsageSnapshot>(()=>ledger.current!.snapshot()),[error,setError]=useState('');
  useEffect(()=>{let alive=true;const timer=setInterval(()=>{void ledger.current!.flush().then(v=>{if(alive){setValue(old=>JSON.stringify(old)===JSON.stringify(v)?old:v);setError('');}}).catch(e=>{if(alive)setError('云转写用量暂未同步：'+String(e));});},1000);return()=>{alive=false;clearInterval(timer);};},[]);
  return {value,error,begin:()=>ledger.current!.begin(),snapshot:async()=>{const v=await ledger.current!.flush();setValue(old=>JSON.stringify(old)===JSON.stringify(v)?old:v);return v;},restore:(v?:CloudUsageSnapshot|null)=>{ledger.current!.restore(v);setValue(ledger.current!.snapshot());setError('');},reset:()=>{ledger.current!.reset();setValue(ledger.current!.snapshot());setError('');},hasRecords:value.tasks.length>0};
}
