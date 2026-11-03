// Regression: render the production indicator for legacy/missing scores.
const fs = require('node:fs');
const path = require('node:path');
const assert = require('node:assert/strict');
const requireFrontend = require('node:module').createRequire(path.resolve(__dirname, '../frontend/package.json'));
const React = requireFrontend('react');
const { renderToStaticMarkup } = requireFrontend('react-dom/server');
const ts = requireFrontend('typescript');
const source = fs.readFileSync(path.resolve(__dirname, '../frontend/src/components/ConfidenceIndicator.tsx'), 'utf8');
const compiled = ts.transpileModule(source, {compilerOptions:{module:ts.ModuleKind.CommonJS,jsx:ts.JsxEmit.ReactJSX}}).outputText;
const out = {exports:{}};
new Function('module','exports','require',compiled)(out,out.exports,requireFrontend);
const {ConfidenceIndicator}=out.exports;
for (const score of [undefined,null,NaN,Infinity,-1,1.01,0,0.4,0.85,1]) {
  const html=renderToStaticMarkup(React.createElement(ConfidenceIndicator,{confidence:score}));
  assert.ok(!html.includes('%') && !/green|orange|red|yellow/.test(html),'unverified scores must not imply accuracy/quality');
  const legacy=typeof score==='number' && Number.isFinite(score) && score>=0 && score<=1;
  assert.ok(html.includes(legacy?'识别分数未验证，不代表准确率':'未提供识别置信度'));
  assert.equal(renderToStaticMarkup(React.createElement(ConfidenceIndicator,{confidence:score,showIndicator:false})), '');
}
console.log('PASS: missing/invalid scores stay unknown; legacy numbers have no accuracy percentage or quality color; preference respected');
