'use client';
import {useEffect,useState} from 'react';
import {getVersion} from '@tauri-apps/api/app';
export function About(){
    const [version,setVersion]=useState('');
    useEffect(()=>{void getVersion().then(setVersion).catch(()=>{});},[]);
    return <section className="p-6 text-center"><img src="/tingxian.png" alt="" width="64" height="64" className="mx-auto rounded-xl"/><h2 className="mt-3 text-xl font-semibold">听弦</h2><p>听懂弦外之音</p><p className="mt-3 text-xs">v{version} · 基于 Meetily 社区版</p></section>;
}
