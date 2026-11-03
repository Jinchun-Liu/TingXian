#!/usr/bin/env python3
"""Report deterministic ASR comparisons without claiming live-call acceptance."""
import argparse,json,math,pathlib
p=argparse.ArgumentParser();p.add_argument('--base',type=pathlib.Path,required=True);p.add_argument('--small',type=pathlib.Path,required=True);p.add_argument('--out',type=pathlib.Path,required=True);p.add_argument('--refined',type=pathlib.Path);a=p.parse_args()
def read(path,expected=16):
 rows=[json.loads(line) for line in path.read_text().splitlines() if line.strip()]
 cases=[r for r in rows if r.get('type')=='case']
 if len(cases)!=expected or len({(r['id'],r['mode']) for r in cases})!=expected:raise ValueError('Expected eight fixtures in both whole/live modes')
 if any(r['audio_s']<1 or not r['text'].strip() for r in cases):raise ValueError('Empty fixture/transcript; manual failure review required')
 return cases
base,small=read(a.base),read(a.small)
refined=read(a.refined,8) if a.refined else []
if refined and ({r['id'] for r in refined}!={r['id'] for r in base} or any(r['mode']!='refined' for r in refined)):raise ValueError('Invalid refined comparison')
if {(r['id'],r['reference']) for r in base}!={(r['id'],r['reference']) for r in small}:raise ValueError('Different reference corpora')
def stats(rows,mode):
 r=[x for x in rows if x['mode']==mode];e=sum(x['character_errors'] for x in r);n=sum(x['reference_characters'] for x in r)
 terms=sum(x['terms'] for x in r);missing=sum(len(x['missing_terms']) for x in r)
 times=sorted(s['inference_ms'] for x in r for s in x['segments'])
 return e/n,terms-missing,terms,sum(x['inference_ms'] for x in r)/1000,times[max(0,math.ceil(.95*len(times))-1)]
lines=['# Whisper base / small 本机离线对照','',
'本报告使用 macOS Tingting 合成的 8 组固定中文夹英文语音。两种模型串行运行同一生产 Whisper 参数路径；整段输入与真实 Silero 实时切片分别比较。没有使用现场录音，没有调用 DeepSeek，没有更改个人资料库。','',
'**界限：** 字符错误率在去标点、空格及大小写差异后计算；未合并繁简、同义词和中英文数字。关键词仅统计原样保留，不能代表语义准确率。推理耗时不含语音采集、停顿等待、检索或云端回答。此项不替代飞书实际口音、通话噪声、30 次端到端延迟或 60 分钟稳定性验收。','',
'## 汇总','',
'| 模型 | 输入方式 | 字符错误率 | 关键词原样保留 | 总推理秒数 | 单片推理 P95 ms |',
'|---|---|---:|---:|---:|---:|']
for name,rows in [('base',base),('small',small)]:
 for mode in ['whole','live']:
  cer,hit,terms,seconds,p95=stats(rows,mode)
  lines.append(f'| {name} | {"整段" if mode=="whole" else "实时切片"} | {cer:.1%} | {hit}/{terms} | {seconds:.2f} | {p95} |')
if refined:
 cer,hit,terms,seconds,p95=stats(refined,'refined')
 lines.append(f'| small | 实时 + 结束整段复核 | {cer:.1%} | {hit}/{terms} | {seconds:.2f} | {p95} |')
lines+=['','## 实时切片逐题比较','','| 题目 | base 实时字符错误率 | small '+('结束复核' if refined else '实时')+'字符错误率 | base 漏失关键词 | small 漏失关键词 |','|---|---:|---:|---|---|']
b={r['id']:r for r in base if r['mode']=='live'};s={r['id']:r for r in (refined or small) if r['mode'] in ('refined','live')}
for key,x in b.items():
 y=s[key];lines.append(f'| {key} | {x["character_errors"]/x["reference_characters"]:.1%} | {y["character_errors"]/y["reference_characters"]:.1%} | {"、".join(x["missing_terms"]) or "无"} | {"、".join(y["missing_terms"]) or "无"} |')
lines+=['','## 原文与结果（供人工核对否定词、数字、术语和漏句）','']
for key,x in b.items():
 lines += [f'### {key}','','参考：'+x['reference'],'','base：'+x['text'],'','small（'+('结束复核' if refined else '实时')+'）：'+s[key]['text'],'']
lines+=['## 复现与来源','','模型来源：[whisper.cpp small 模型](https://huggingface.co/ggerganov/whisper.cpp/blob/main/ggml-small.bin)，发布 SHA-256：`1be3a9b2063867b937e64e2ec7483364a79917e157fa98c5d94b5c1fffea987b`。下载后应校验，不以文件名或大小替代校验。','',
'固定题集：`interview/asr-comparison-cases.json`；生成脚本：`scripts/prepare-asr-comparison.py`；对照程序：`frontend/src-tauri/examples/compare_interview_asr.rs`。完整逐片结果和输入波形保留在隔离工具目录，未打包到应用。','']
a.out.write_text('\n'.join(lines));print(a.out)
