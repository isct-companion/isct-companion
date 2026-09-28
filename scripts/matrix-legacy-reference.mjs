// Test-only reconstruction of the pre-Rust TS pipeline from its recorded source.
// In particular, header grouping retains the original four-row gap rule.
import { filter } from './matrix-filter-reference.mjs';
export const project=(corners,u,v)=>{
 const [a,b,c,d]=corners,dx=a.x-b.x+c.x-d.x,dy=a.y-b.y+c.y-d.y,bx=b.x-c.x,by=b.y-c.y,cx=d.x-c.x,cy=d.y-c.y;
 const det=bx*cy-cx*by,g=Math.abs(det)<1e-8?0:(dx*cy-cx*dy)/det,h=Math.abs(det)<1e-8?0:(bx*dy-dx*by)/det,z=g*u+h*v+1;
 return {x:((b.x-a.x+g*b.x)*u+(d.x-a.x+h*d.x)*v+a.x)/z,y:((b.y-a.y+g*b.y)*u+(d.y-a.y+h*d.y)*v+a.y)/z};
};
const median=v=>[...v].sort((a,b)=>a-b)[Math.floor(v.length/2)];
function fit(points){const slopes=[];for(let i=0;i<points.length;i++)for(let j=i+1;j<points.length;j++)if(Math.abs(points[j].x-points[i].x)>10)slopes.push((points[j].y-points[i].y)/(points[j].x-points[i].x));const slope=slopes.length?median(slopes):0,offset=median(points.map(p=>p.y-slope*p.x));return x=>offset+slope*x;}
function template(image){
 const {width:w,height:h,data}=image;
 const dark=(x,y,t=125)=>{x=Math.round(x);y=Math.round(y);if(x<0||y<0||x>=w||y>=h)return false;const i=(y*w+x)*4;return Math.max(data[i],data[i+1],data[i+2])<t;};
 const runs=[],gap=Math.max(3,Math.round(w/70));
 for(let y=0;y<h;y++){let left=-1,last=-1;for(let x=0;x<=w+gap;x++){if(x<w&&dark(x,y)){if(left<0)left=x;last=x;}else if(left>=0&&x-last>gap){if(last-left>w*.2)runs.push({y,left,right:last});left=-1;}}}
 const groups=[];for(const run of runs){const group=groups.find(g=>run.y-g.at(-1).y<=4&&run.y>=g.at(-1).y&&Math.abs(run.left-g.at(-1).left)<w*.06&&Math.abs(run.right-g.at(-1).right)<w*.06);if(group)group.push(run);else groups.push([run]);}
 const candidates=groups.filter(g=>{const span=median(g.map(r=>r.right-r.left)),height=g.at(-1).y-g[0].y+1;return span/height>7&&span/height<28&&height>=6;}).sort((a,b)=>b.length*median(b.map(r=>r.right-r.left))-a.length*median(a.map(r=>r.right-r.left)));
 for(const group of candidates){
  const left=median(group.map(r=>r.left)),right=median(group.map(r=>r.right)),cw=(right-left)/10,mid=median(group.map(r=>r.y)),tops=[],bottoms=[];
  for(let x=left+cw*.1;x<right-cw*.1;x+=Math.max(2,cw*.17)){let top=mid,bottom=mid;while(top>group[0].y-cw*.2&&dark(x,top))top--;while(bottom<group.at(-1).y+cw*.2&&dark(x,bottom))bottom++;if(bottom-top>cw*.3){tops.push({x,y:top+1});bottoms.push({x,y:bottom});}}
  if(bottoms.length<10)continue;const topLine=fit(tops),startLine=fit(bottoms),hh=startLine((left+right)/2)-topLine((left+right)/2);if(hh<cw*.4||hh>cw*1.1)continue;
  const ends=[];for(const f of [.15,.25,.75,.85]){const x=left-cw*f,start=startLine(x)+hh*.3;let last=start,missing=0,hits=0;for(let y=Math.round(start);y<Math.min(h,start+hh*9);y++){if(dark(x,y,190)){last=y;missing=0;hits++;}else missing++;if(missing>hh*.45)break;}const extent=last-startLine(x);if(extent>hh*5&&extent<hh*8.5&&hits/extent>.65)ends.push({x,y:last});}
  if(ends.length<2)continue;const bl=median(ends.map(p=>p.y)),bh=bl-startLine(left),edges=[];
  for(let y=startLine(right)+bh*.15;y<bl-bh*.05;y+=Math.max(2,bh/30))for(let x=Math.round(right+cw*.35);x>right-cw*.25;x--)if(dark(x,y,155)){edges.push({x:y,y:x});break;}
  if(edges.length<10)continue;const rightEdge=fit(edges),be=[];
  for(let x=left+cw*.3;x<right-cw*.3;x+=cw*.4)for(let y=Math.round(bl+hh*.3);y>bl-hh*.3;y--)if(dark(x,y,165)){be.push({x,y});break;}
  const bottomLine=be.length>=10?fit(be):undefined,brx=rightEdge(bottomLine?bottomLine(right):bl),slope=(startLine(right)-startLine(left))/(right-left);
  const corners=[{x:left,y:startLine(left)},{x:right,y:startLine(right)},{x:brx,y:bottomLine?bottomLine(brx):bl+slope*(brx-left)},{x:left,y:bottomLine?bottomLine(left):bl}];
  if(corners.some(p=>p.x<0||p.y<0||p.x>=w||p.y>=h))continue;return corners;
 }
}
function lines(image,vertical,count){
 const length=vertical?image.width:image.height,depth=vertical?image.height:image.width,best=Array.from({length},(_,position)=>({position,slope:0,support:0})),edges=best.map(l=>({...l}));
 for(let step=-16;step<=16;step++){const slope=step/100,scores=new Float32Array(length),es=new Float32Array(length);for(let cross=0;cross<depth;cross++){const shift=Math.round(slope*(cross-depth/2));for(let axis=0;axis<length;axis++){const x=vertical?axis:cross,y=vertical?cross:axis,i=(y*image.width+x)*4;if(Math.max(image.data[i],image.data[i+1],image.data[i+2])>125)continue;const p=axis-shift;if(p>=0&&p<length){scores[p]++;const next=vertical?i+12:i+image.width*12;if(axis+3<length&&Math.max(image.data[next],image.data[next+1],image.data[next+2])>160)es[p]++;}}}for(let p=0;p<length;p++){if(es[p]/depth>edges[p].support)edges[p]={position:p,slope,support:es[p]/depth};if(scores[p]/depth>best[p].support)best[p]={position:p,slope,support:scores[p]/depth};}}
 const groups=[];for(const line of best){if(line.support<.38)continue;const last=groups.at(-1);if(last&&line.position<=last.at(-1).position+2)last.push(line);else groups.push([line]);}
 const candidates=groups.flatMap(g=>{const broad=g.at(-1).position-g[0].position>=length/(count*2);if(!broad)return[g.reduce((a,b)=>a.support>=b.support?a:b)];const edge=g.map(l=>edges[l.position]).reduce((a,b)=>a.support>=b.support?a:b);return edge.support>=.38?[edge]:[];});
 let result,score=-Infinity;for(let start=0;start<=candidates.length-count;start++){const ls=candidates.slice(start,start+count),gap=(ls.at(-1).position-ls[0].position)/(count-1);if(gap<length/(count*2))continue;const error=Math.max(...ls.map((l,i)=>Math.abs(l.position-ls[0].position-i*gap)))/gap;if(error>.18)continue;const s=ls.reduce((sum,l)=>sum+l.support,0)/count-error;if(s>score){score=s;result=ls;}}return result;
}
export function locate(image){
 const t=template(image);if(t)return Array.from({length:70},(_,i)=>{const c=i%10,r=Math.floor(i/10);return[[c/10,r/7],[(c+1)/10,r/7],[(c+1)/10,(r+1)/7],[c/10,(r+1)/7]].map(([u,v])=>project(t,u,v));});
 const vs=lines(image,true,11),hs=lines(image,false,8);if(!vs||!hs)return;
 const corner=(c,r)=>{const v=vs[c],h=hs[r],vx=v.position-v.slope*image.height/2,hy=h.position-h.slope*image.width/2,x=(vx+v.slope*hy)/(1-v.slope*h.slope);return{x,y:hy+h.slope*x};};
 for(let r=0;r<8;r++)for(let c=0;c<11;c++){const p=corner(c,r);if(p.x<0||p.y<0||p.x>=image.width||p.y>=image.height||(c&&p.x-corner(c-1,r).x<4)||(r&&p.y-corner(c,r-1).y<4))return;}
 return Array.from({length:70},(_,i)=>{const c=i%10,r=Math.floor(i/10);return[corner(c,r),corner(c+1,r),corner(c+1,r+1),corner(c,r+1)];});
}
export function crop(image,corners){const rgb=new Uint8Array(64*64*3);for(let y=0;y<64;y++)for(let x=0;x<64;x++){const p=project(corners,.09+x/63*.82,.09+y/63*.82),sx=Math.min(image.width-1,Math.max(0,Math.round(p.x))),sy=Math.min(image.height-1,Math.max(0,Math.round(p.y))),input=(sy*image.width+sx)*4;rgb.set(image.data.subarray(input,input+3),(y*64+x)*3);}return rgb;}
export function prepare(data,width=64,height=64){
 let left=width,top=height,right=-1,bottom=-1;for(let y=0;y<height;y++)for(let x=0;x<width;x++)if(data[y*width+x]<128){left=Math.min(left,x);top=Math.min(top,y);right=Math.max(right,x);bottom=Math.max(bottom,y);}
 if(right<left){left=0;top=0;right=width-1;bottom=height-1;}const w=right-left+17,h=bottom-top+17,rw=Math.min(320,Math.ceil(48*w/h)),plane=48*320,tensor=new Float32Array(3*plane),preview=new Uint8Array(plane).fill(128);
 const value=(x,y)=>x<8||y<8||x>=w-8||y>=h-8?255:data[(y-8+top)*width+x-8+left];
 for(let y=0;y<48;y++)for(let x=0;x<rw;x++){const sx=Math.max(0,Math.min(w-1,(x+.5)*w/rw-.5)),sy=Math.max(0,Math.min(h-1,(y+.5)*h/48-.5)),x0=Math.floor(sx),y0=Math.floor(sy),x1=Math.min(w-1,x0+1),y1=Math.min(h-1,y0+1),fx=sx-x0,fy=sy-y0,pixel=Math.round((value(x0,y0)*(1-fx)+value(x1,y0)*fx)*(1-fy)+(value(x0,y1)*(1-fx)+value(x1,y1)*fx)*fy),o=y*320+x;preview[o]=pixel;for(let c=0;c<3;c++)tensor[c*plane+o]=pixel/127.5-1;}
 return {tensor,preview};
}
export function decode(data,characters){let previous=-1,text='',confidence=0,count=0;for(let i=0;i<data.length;i+=characters.length){let best=0,score=-Infinity;for(let c=0;c<characters.length;c++)if(data[i+c]>score){best=c;score=data[i+c];}if(best!==0&&best!==previous){text+=characters[best];confidence+=score;count++;}previous=best;}return{text,confidence:count?confidence/count:0};}
const digit={0:'O',1:'I',8:'B'},letter=t=>digit[t.trim()]??(/^[A-Z]$/.test(t.trim())?t.trim():'');
export function accept(text,confidence){const l=letter(text);return !l||!Number.isFinite(confidence)||confidence<.65||confidence>1?{text:'',confidence:0}:{text:l,confidence:digit[text.trim()]?Math.min(confidence,.89):confidence};}
export function review(cleaned,original){if(!cleaned.text)return cleaned;if(cleaned.text!==letter(original))return{...cleaned,confidence:Math.min(cleaned.confidence,.74)};return digit[original.trim()]?{...cleaned,confidence:Math.min(cleaned.confidence,.89)}:cleaned;}
export {filter};
