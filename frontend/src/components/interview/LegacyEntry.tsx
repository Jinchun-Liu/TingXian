'use client';
import {useEffect} from 'react';
import {useRouter} from 'next/navigation';
export function LegacyEntry({panel='history'}:{panel?:'audio'|'history'}){
  const router=useRouter();
  useEffect(()=>{
    const query=new URLSearchParams({panel});
    const id=new URLSearchParams(window.location.search).get('id');
    if(panel==='history'&&id)query.set('meeting',id);
    router.replace('/interview-workspace?'+query.toString());
  },[panel,router]);
  return <main className="p-4" aria-busy="true">正在打开听弦…</main>;
}
