// Explicit private-photo regression. Binds only to loopback; prints aggregates.
// Temporary images/OCR strings live in a 0700 directory, deleted in finally.
import fs from 'node:fs';
import path from 'node:path';
import http from 'node:http';
import {spawn} from 'node:child_process';
const { chromium } = await import(process.env.PLAYWRIGHT_MODULE ?? 'playwright');
process.umask(0o077);
if (!process.argv[2]) throw new Error('Pass the local private photo path.');
const root=process.cwd(),photo=path.resolve(process.argv[2]);
const ortDir=process.env.ORT_WEB_DIST;
if(!ortDir)throw new Error('Set ORT_WEB_DIST to local onnxruntime-web 1.24.3 dist');
const scratch=fs.mkdtempSync('/private/tmp/isct-private-parity-');fs.chmodSync(scratch,0o700);
const write=(name,data)=>fs.writeFileSync(path.join(scratch,name),data,{mode:0o600});
let failed=false;
const production=fs.readFileSync('src/core/matrix-ocr.ts','utf8');
const prepare=production.slice(production.indexOf('export async function prepareMatrixCanvases'),production.indexOf('export async function recognizeMatrixImage')).replace('file: File','file');
const page=`<!doctype html><script>addEventListener('error',()=>fetch('/failed',{method:'POST',body:'module'}));addEventListener('unhandledrejection',()=>fetch('/failed',{method:'POST',body:'rejection'}));</script><script type="module">
import * as ort from '/ort/ort.wasm.min.mjs';
import * as legacy from '/legacy.mjs';
import {prepareMatrixCanvases} from '/prepare.mjs';
try {
 if(new URL(location.href).searchParams.has('prepare')) {
 const blob=await(await fetch('/photo')).blob();
 const images=await prepareMatrixCanvases(new File([blob],'private.png',{type:'image/png'}));
 await fetch('/prepared',{method:'POST',body:JSON.stringify(images)});
 } else {
 const images=await(await fetch('/prepared')).json();
 const raster=async(data)=>{const bm=await createImageBitmap(await(await fetch('data:image/png;base64,'+data)).blob());const c=document.createElement('canvas');c.width=bm.width;c.height=bm.height;const ctx=c.getContext('2d');ctx.drawImage(bm,0,0);bm.close();return ctx.getImageData(0,0,c.width,c.height);};
 const source=await raster(images.data),small=await raster(images.preview);
 const old=legacy.locate(small);
 let native;
 for(;;){const response=await fetch('/native-grid');if(response.status===200){native=await response.json();break;}await new Promise(r=>setTimeout(r,200));}
 const grid=old??native;if(!grid)throw new Error('no-grid');
 ort.env.wasm.numThreads=1;ort.env.wasm.wasmPaths=location.origin+'/ort/';
 const session=await ort.InferenceSession.create('/model',{executionProviders:['wasm'],graphOptimizationLevel:'all'});
 const chars=await(await fetch('/characters')).json();
 const recognize=async(tensor)=>{const input=new ort.Tensor('float32',tensor,[1,3,48,320]);const output=await session.run({[session.inputNames[0]]:input});try{return legacy.decode(output[session.outputNames[0]].data,chars);}finally{input.dispose();Object.values(output).forEach(x=>x.dispose());}};
 const hash=async(bytes)=>Array.from(new Uint8Array(await crypto.subtle.digest('SHA-256',bytes))).map(n=>n.toString(16).padStart(2,'0')).join('');
 const cells=[];
 for(const points of grid){
  const crop=legacy.crop(source,points.map(p=>({x:p.x*source.width/small.width,y:p.y*source.height/small.height})));
  const binary=legacy.filter(64,64,crop),prepared=legacy.prepare(binary.cleaned);
  let raw={text:'',confidence:0},accepted={text:'',confidence:0};
  if(binary.ink>=12&&binary.ink<=4096*.55){raw=await recognize(prepared.tensor);accepted=legacy.accept(raw.text,raw.confidence);if(accepted.text&&binary.original.some((v,i)=>v!==binary.cleaned[i]))accepted=legacy.review(accepted,(await recognize(legacy.prepare(binary.original).tensor)).text);}
  cells.push({crop_hash:await hash(crop),binary_hash:await hash(binary.original),clean_hash:await hash(binary.cleaned),tensor_hash:await hash(prepared.tensor.buffer),raw,accepted});
 }
 await session.release();
 await fetch('/reference',{method:'POST',body:JSON.stringify({legacy_found:!!old,grid,cells})});
}
} catch(_) {await fetch('/failed',{method:'POST',body:'failed'});}
</script>`;
const server=http.createServer(async(req,res)=>{
 try{
 if(req.method==='POST'){
  if(!['/prepared','/reference','/failed'].includes(req.url)){res.writeHead(404).end();return;}
  let chunks=[],size=0;for await(const chunk of req){size+=chunk.length;if(size>40*1024*1024)throw new Error('size');chunks.push(chunk);}
  if(req.url==='/failed')failed=true;else write(req.url.slice(1)+'.json',Buffer.concat(chunks));res.end('ok');return;
 }
 let data,type='application/javascript';
 if(req.url==='/'){data=page;type='text/html';}
 else if(req.url==='/?prepare'){data=page;type='text/html';}
 else if(req.url==='/prepared'){data=fs.readFileSync(path.join(scratch,'prepared.json'));type='application/json';}
 else if(req.url==='/photo'){data=fs.readFileSync(photo);type='image/png';}
 else if(req.url==='/prepare.mjs')data=prepare;
 else if(req.url==='/legacy.mjs')data=fs.readFileSync('scripts/matrix-legacy-reference.mjs');
 else if(req.url==='/matrix-filter-reference.mjs')data=fs.readFileSync('scripts/matrix-filter-reference.mjs');
 else if(req.url==='/model'){data=fs.readFileSync('backend/models/paddle/en-ppocrv5.onnx');type='application/octet-stream';}
 else if(req.url==='/characters'){data=fs.readFileSync('backend/models/paddle/characters.json');type='application/json';}
 else if(req.url==='/native-grid'){if(!fs.existsSync(path.join(scratch,'native-grid.json'))){res.writeHead(202).end();return;}data=fs.readFileSync(path.join(scratch,'native-grid.json'));type='application/json';}
 else if(/^\/ort\/[a-zA-Z0-9_.-]+$/.test(req.url)){data=fs.readFileSync(path.join(ortDir,path.basename(req.url)));if(req.url.endsWith('.wasm'))type='application/wasm';}
 else{res.writeHead(404).end();return;}
 res.setHeader('Content-Type',type);res.setHeader('Cache-Control','no-store');res.end(data);
 }catch(_){failed=true;res.writeHead(500).end('local test error');}
});
const waitFor=async name=>{const start=Date.now();while(!fs.existsSync(path.join(scratch,name))){if(failed)throw new Error('Private browser stage failed; no sensitive output emitted.');if(Date.now()-start>120000)throw new Error('Private browser stage timed out.');await new Promise(r=>setTimeout(r,200));}};
const cargo=stage=>new Promise((resolve,reject)=>{
 const child=spawn('cargo',['test','--manifest-path','backend/Cargo.toml','--locked','private_ts_native_parity','--','--ignored','--nocapture'],{cwd:root,env:{...process.env,MATRIX_PRIVATE_DIR:scratch,MATRIX_PRIVATE_STAGE:stage},stdio:['ignore','pipe','pipe']});
 // Discard raw tool output: failing test diagnostics must never leak characters.
 child.stdout.resume();child.stderr.resume();child.on('exit',code=>code===0?resolve():reject(new Error('Native private comparison failed (details suppressed).')));
});
let webkit,browser;
try{
 await new Promise(r=>server.listen(0,'127.0.0.1',r));const url='http://127.0.0.1:'+server.address().port+'/';
 write('runner.swift',`import Cocoa\nimport WebKit\nlet app=NSApplication.shared\napp.setActivationPolicy(.prohibited)\nlet web=WKWebView(frame:NSRect(x:0,y:0,width:600,height:500))\nweb.load(URLRequest(url:URL(string:CommandLine.arguments[1])!))\napp.run()\n`);
 webkit=spawn('swift',[path.join(scratch,'runner.swift'),url+'?prepare'],{stdio:'ignore'});
 await waitFor('prepared.json');console.log('Private WebKit image preparation complete.');webkit.kill('SIGTERM');
 await cargo('prepare');console.log('Native geometry stage complete.');
 browser=await chromium.launch({headless:true});const tab=await browser.newPage();await tab.route('**/*',route=>{const target=route.request().url();return target.startsWith(url)||target.startsWith('data:')||target.startsWith('blob:')?route.continue():route.abort();});await tab.goto(url);
 await waitFor('reference.json');console.log('Legacy TS/WASM stage complete.');
 try {await cargo('compare');} finally {if(fs.existsSync(path.join(scratch,'summary.json')))console.log(fs.readFileSync(path.join(scratch,'summary.json'),'utf8'));}
} finally {
 await browser?.close();webkit?.kill('SIGTERM');server.closeAllConnections();server.close();fs.rmSync(scratch,{recursive:true,force:true});
}
