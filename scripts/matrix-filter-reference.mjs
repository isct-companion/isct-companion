// Frozen JavaScript reference for the original TS cell filter. Test-only, no model.
// Retains Float32Array assignments and Number arithmetic from the original pipeline.
export function filter(width, height, rgb) {
  const gray = new Float32Array(width * height);
  for (let i=0;i<gray.length;i++) gray[i]=rgb[i*3]*.299+rgb[i*3+1]*.587+rgb[i*3+2]*.114;
  const sharp=new Float32Array(gray.length),kernel=[1,2,1];
  for(let y=0;y<height;y++)for(let x=0;x<width;x++) {
    let blurred=0;
    for(let dy=-1;dy<=1;dy++)for(let dx=-1;dx<=1;dx++) {
      const sx=Math.max(0,Math.min(width-1,x+dx)),sy=Math.max(0,Math.min(height-1,y+dy));
      blurred+=gray[sy*width+sx]*kernel[dx+1]*kernel[dy+1]/16;
    }
    const i=y*width+x;sharp[i]=Math.max(0,Math.min(255,gray[i]+(gray[i]-blurred)));
  }
  const stride=width+1,integral=new Float64Array(stride*(height+1));
  for(let y=0;y<height;y++){let sum=0;for(let x=0;x<width;x++){sum+=sharp[y*width+x];integral[(y+1)*stride+x+1]=integral[y*stride+x+1]+sum;}}
  const original=new Uint8Array(width*height);
  for(let y=0;y<height;y++)for(let x=0;x<width;x++){
    const left=Math.max(0,x-7),right=Math.min(width,x+8),top=Math.max(0,y-7),bottom=Math.min(height,y+8);
    const mean=(integral[bottom*stride+right]-integral[top*stride+right]-integral[bottom*stride+left]+integral[top*stride+left])/((right-left)*(bottom-top));
    original[y*width+x]=sharp[y*width+x]<mean-10?0:255;
  }
  const cleaned=new Uint8Array(original),visited=new Uint8Array(width*height);
  let ink=0;
  for(let seed=0;seed<visited.length;seed++){
    if(visited[seed]||cleaned[seed]>=128)continue;
    const component=[seed];visited[seed]=1;
    for(let head=0;head<component.length;head++){
      const p=component[head],x=p%width,y=Math.floor(p/width);
      for(let dy=-1;dy<=1;dy++)for(let dx=-1;dx<=1;dx++){
        const nx=x+dx,ny=y+dy;if(nx<0||ny<0||nx>=width||ny>=height)continue;
        const next=ny*width+nx;if(!visited[next]&&cleaned[next]<128){visited[next]=1;component.push(next);}
      }
    }
    if(component.length<8)for(const pixel of component)cleaned[pixel]=255;else ink+=component.length;
  }
  return {original,cleaned,ink};
}

// Deterministic, non-private fixtures used by the native parity regression.
export function fixture(width,height,seed) {
 const rgb=new Uint8Array(width*height*3);
 let state=seed;
 for(let i=0;i<rgb.length;i++){state=(Math.imul(state,1664525)+1013904223)>>>0;rgb[i]=state>>>24;}
 // Mix a red/gray background, thin glyph strokes and detached noise.
 for(let y=0;y<height;y++)for(let x=0;x<width;x++){
  if(x<width/2){const p=(y*width+x)*3;const ink=x===Math.floor(width/4)||y===Math.floor(height/2);rgb[p]=ink?20:220;rgb[p+1]=ink?20:150;rgb[p+2]=ink?20:150;}
 }
 return rgb;
}
if(typeof process !== 'undefined' && process.argv[1]?.endsWith('matrix-filter-reference.mjs')) {
 const {createHash}=await import('node:crypto');const fs=await import('node:fs');
 const hash=v=>createHash('sha256').update(v).digest('hex');
 const cases=[];
 for(const [w,h]of [[8,9],[32,32],[64,64],[63,65]])for(const seed of [1,17,255,314159]) {
  const r=filter(w,h,fixture(w,h,seed));cases.push({width:w,height:h,seed,original:hash(r.original),cleaned:hash(r.cleaned),ink:r.ink});
 }
 fs.mkdirSync('backend/src/matrix_ocr/fixtures',{recursive:true});
 fs.writeFileSync('backend/src/matrix_ocr/fixtures/ts-filter.json',JSON.stringify(cases,null,2)+'\n');
}
