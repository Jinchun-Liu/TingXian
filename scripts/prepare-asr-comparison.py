#!/usr/bin/env python3
"""Create synthetic offline fixtures. No microphone, API or user recordings."""
import argparse,json,pathlib,subprocess
parser=argparse.ArgumentParser();parser.add_argument('--out',type=pathlib.Path,required=True);parser.add_argument('--ffmpeg',type=pathlib.Path,required=True)
args=parser.parse_args();root=pathlib.Path(__file__).resolve().parents[1]
source=json.loads((root/'interview/asr-comparison-cases.json').read_text())
args.out.mkdir(parents=True,exist_ok=True)
cases=[]
for case in source['cases']:
    stem=args.out/case['id'];aiff=stem.with_suffix('.aiff');pcm=stem.with_suffix('.f32')
    # Do not silently overwrite existing fixtures: a repeat must use a new folder.
    if aiff.exists() or pcm.exists():raise SystemExit(f'Fixture already exists; choose a fresh output folder: {stem}')
    subprocess.run(['say','-v',source['voice'],'-r',str(source['rate']),'-o',str(aiff),case['text']],check=True)
    subprocess.run([str(args.ffmpeg),'-nostdin','-v','error','-i',str(aiff),'-ac','1','-ar','16000','-f','f32le',str(pcm)],check=True)
    if pcm.stat().st_size < 16000*4:
        raise SystemExit('System speech service produced empty/short audio; no usable fixture created. Retry with speech-service access and a fresh folder.')
    cases.append({**case,'pcm':str(pcm.resolve()),'audio_seconds':pcm.stat().st_size/4/16000})
manifest={**source,'cases':cases}
(args.out/'manifest.json').write_text(json.dumps(manifest,ensure_ascii=False,indent=2))
print(json.dumps({'fixture_count':len(cases),'total_audio_seconds':round(sum(c['audio_seconds'] for c in cases),2),'manifest':str(args.out/'manifest.json')},ensure_ascii=False))
