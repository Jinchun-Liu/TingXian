// Run every deterministic interview UI regression. No API credentials required.
const fs=require('node:fs'),path=require('node:path'),{spawnSync}=require('node:child_process');
const root=path.resolve(__dirname,'..');
const files=fs.readdirSync(__dirname).filter(name=>/^test-interview(?:-.+)?\.cjs$/.test(name) || name==='test-transcript-confidence.cjs').sort();
let failed=0;
for(const name of files){const result=spawnSync(process.execPath,[path.join(__dirname,name)],{cwd:root,stdio:'inherit'});if(result.status!==0)failed++;}
console.log(`${files.length} test scripts, ${failed} failed`);process.exitCode=failed?1:0;
