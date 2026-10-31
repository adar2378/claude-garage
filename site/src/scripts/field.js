import { $, clamp, reduce } from './util.js';

/* Hero: ASCII tile field on canvas. */
export function createField() {
  var Field;
  var cv=$('#field'), ctx=cv.getContext('2d'), hero=$('#hero');
  var W=0,H=0,dpr=1,cw=7.2,ch=17,fs=12,tiles=[],amber=null,t0=0,vis=true,raf=0,attached=false,ptr={x:-999,y:-999},rips=[],lastAmb=0,hover=false,reveal=reduce?1:0,rects=[],ready=false;
  var TW=10,TH=5,GX=3,GY=1;
  var SETS=[['┌','─','┐','│','└','┘'],['┏','━','┓','┃','┗','┛'],['╔','═','╗','║','╚','╝']];
  var INK='26,27,33';
  function rng(a){return function(){a|=0;a=a+0x6D2B79F5|0;var t=Math.imul(a^a>>>15,1|a);t=t+Math.imul(t^t>>>7,61|t)^t;return((t^t>>>14)>>>0)/4294967296}}
  function sm(x){x=clamp(x,0,1);return x*x*(3-2*x)}
  function rel(el){var a=el.getBoundingClientRect(),b=hero.getBoundingClientRect();return{x:a.left-b.left,y:a.top-b.top,w:a.width,h:a.height}}
  function dist(T,r){var dx=Math.max(r.x-(T.x+T.w),T.x-(r.x+r.w),0),dy=Math.max(r.y-(T.y+T.h),T.y-(r.y+r.h),0);return Math.hypot(dx,dy)}
  function collect(){
    rects=[];
    [['.pill',.12],['h1',.05],['.sub',.07],['.copybox',.4],['.cta .btn',.4],['.note',.2]].forEach(function(p){var el=$(p[0],hero);if(el){var r=rel(el);r.wt=p[1];rects.push(r)}});
    var w=$('#hwin');if(w){var r=rel(w);r.wt=0;rects.push(r)}
  }
  function build(){
    W=hero.clientWidth;H=hero.clientHeight;dpr=Math.min(window.devicePixelRatio||1,2);
    cv.width=Math.round(W*dpr);cv.height=Math.round(H*dpr);ctx.setTransform(dpr,0,0,dpr,0,0);
    fs=W<700?10:12;ctx.font='400 '+fs+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';cw=ctx.measureText('M').width;ch=Math.round(fs*1.42);
    collect();
    var px=(TW+GX)*cw, py=(TH+GY)*ch, rnd=rng(7);
    var cols=Math.ceil(W/px)+2, rows=Math.ceil(H/py)+1;
    var bars=['▰▱▱▱','▰▰▱▱','▰▰▰▱','▰▰▰▰'];
    tiles=[];
    for(var r=0;r<rows;r++)for(var c=-1;c<cols;c++){
      var x=c*px+(r%2?px/2:0)-cw*2, y=r*py+18;
      var T={x:x,y:y,w:TW*cw,h:TH*ch,cx:x+TW*cw/2,cy:y+TH*ch/2,pop:-1e9,seed:rnd(),md:1e9};
      T.working=rnd()<.38; T.pet=rnd()<.17;
      var r1='';for(var k=0;k<8;k++)r1+=rnd()<.62?'▪':' ';
      T.r1=r1; T.bar=bars[Math.floor(rnd()*4)];
      var f=1;
      for(var i=0;i<rects.length;i++){var d=dist(T,rects[i]);T.md=Math.min(T.md,d);var wt=rects[i].wt;f=Math.min(f,wt+(1-wt)*sm(d/110))}
      T.k=f;
      tiles.push(T);
    }
    var tx=W<700?W*.62:W-190, ty=W<700?52:H*.27, best=1e9; amber=null;
    tiles.forEach(function(T){if(T.md<24||T.y+T.h>H-4)return;var d=Math.hypot(T.cx-tx,T.cy-ty);if(d<best&&T.x>-cw&&T.x+T.w<W+cw){best=d;amber=T}});
    if(!amber){tiles.forEach(function(T){if(!amber||T.md>amber.md)amber=T})}
    if(amber){amber.k=1;amber.isAmber=true}
    ready=true;
  }
  function ripple(x,y,s){rips.push({x:x,y:y,t:performance.now(),s:s||1});if(rips.length>8)rips.shift()}
  function intensity(T,now){
    var I=0;
    for(var i=0;i<rips.length;i++){var R=rips[i],age=(now-R.t)/1000;if(age>2.6)continue;var d=Math.hypot(T.cx-R.x,T.cy-R.y),rad=age*560,z=(d-rad)/80;I+=Math.exp(-z*z)*Math.max(0,1-age/2.6)*R.s}
    var pd=Math.hypot(T.cx-ptr.x,T.cy-ptr.y); if(pd<170)I+=(1-pd/170)*.55;
    var pa=(now-T.pop)/1000; if(pa<1.6)I+=Math.max(0,1-pa/1.6)*1.1;
    return I;
  }
  function drawTile(T,I,now){
    var popping=(now-T.pop)<2400;
    var k=popping?1:T.k;
    var a=(.17+.4*Math.min(I,1.2))*k;
    var g=I<.28?0:I<.62?1:2, S=SETS[g], hot=I>.28;
    ctx.fillStyle=hot?'rgba(232,80,2,'+Math.min(a*1.25,.62)+')':'rgba('+INK+','+Math.min(a,.55)+')';
    var m=S[1].repeat(TW-2), x=T.x, y=T.y;
    ctx.fillText(S[0]+m+S[2],x,y);
    var awake=I>.55||popping;
    var rows=[' '+' '+' '+T.bar+'  ',' '+T.r1+' ',T.pet?(awake?'  (0.0)  ':'  =-.-=  '):'   ▪▪▪   '];
    if(popping&&T.pop>0)rows[0]='  ▰▰▰▰  ';
    for(var i=0;i<3;i++)ctx.fillText(S[3]+rows[i]+S[3],x,y+ch*(i+1));
    ctx.fillText(S[4]+m+S[5],x,y+ch*4);
    ctx.fillStyle=T.working?'rgba(232,80,2,'+Math.min(a*2.2+.1,.7)+')':'rgba('+INK+','+Math.min(a*1.6,.5)+')';
    ctx.fillText('●',x+cw*2,y+ch);
  }
  function drawAmber(T,t,now){
    var blink=attached?1:(.62+.38*Math.sin(t*3.6)), col=attached?'232,80,2':'255,176,32';
    var S=SETS[1],m=S[1].repeat(TW-2);
    ctx.save();
    ctx.fillStyle='rgba('+col+','+(attached?.1:.2)+')';
    ctx.fillRect(T.x-cw*.2,T.y-ch*.1,T.w+cw*.4,T.h+ch*.1);
    ctx.shadowColor='rgba('+col+',.8)';ctx.shadowBlur=attached?8:16;
    ctx.fillStyle='rgba('+col+','+(.7+.3*blink)+')';
    ctx.fillText(S[0]+m+S[2],T.x,T.y);
    ctx.fillText(S[4]+m+S[5],T.x,T.y+ch*4);
    ctx.fillText(S[3]+'        '+S[3],T.x,T.y+ch);ctx.fillText(S[3]+'        '+S[3],T.x,T.y+ch*2);ctx.fillText(S[3]+'        '+S[3],T.x,T.y+ch*3);
    ctx.restore();
    ctx.fillStyle=attached?'rgba(179,62,0,.95)':'rgba(107,67,0,.95)';
    var inner=attached?['  ▰▰▰▱  ',' (^.^)  ','  ▪▪▪   ']:['  ▰▰▰▱  ',' (0.0) !','  ▪▪▪   '];
    for(var i=0;i<3;i++){ctx.fillText(inner[i],T.x+cw,T.y+ch*(i+1))}
    ctx.font='500 '+(fs-1)+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';
    var lb=attached?'attached':'needs you';
    ctx.fillText(lb,T.cx-ctx.measureText(lb).width/2,T.y+T.h+8);
    ctx.font='400 '+fs+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';
    if(!attached){var kk=(((now-lastAmb)/1000)%2.6)/2.6, rr=8+kk*46;ctx.strokeStyle='rgba(255,176,32,'+(.4*(1-kk))+')';ctx.lineWidth=1.25;ctx.strokeRect(T.x-rr*.9,T.y-rr*.5,T.w+rr*1.8,T.h+rr)}
  }
  function draw(now,t){
    ctx.clearRect(0,0,W,H);
    ctx.font='400 '+fs+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';ctx.textBaseline='top';
    for(var i=0;i<tiles.length;i++){
      var T=tiles[i]; if(T.x>W||T.x+T.w<0||T.y>H||T.y+T.h<0) continue;
      var popping=(now-T.pop)<2400;
      if(T.k<.02&&!popping&&!T.isAmber) continue;
      var dn=Math.hypot(T.cx-W/2,T.cy-H*.3)/Math.hypot(W/2,H*.7);
      var ra=clamp(reveal*2-dn,0,1); if(ra<=0) continue;
      ctx.globalAlpha=ra;
      if(T.isAmber){drawAmber(T,t,now);continue}
      drawTile(T,intensity(T,now),now);
    }
    ctx.globalAlpha=1;
  }
  function frame(now){
    raf=0; if(!vis||!ready) return;
    if(!t0) t0=now;
    var t=(now-t0)/1000;
    reveal=sm((t-.3)/2.2);
    if(now-lastAmb>6000&&t>.2){ lastAmb=now; ripple(W*(.12+Math.random()*.76),H*(.08+Math.random()*.6),.8) }
    if(t<.05) ripple(W*.5,H*.3,1.1);
    draw(now,t);
    if(amber){
      var ah=Math.abs(ptr.x-amber.cx)<amber.w/2+14&&Math.abs(ptr.y-amber.cy)<amber.h/2+14;
      if(ah!==hover){hover=ah;Field.onHover&&Field.onHover(ah)}
    }
    raf=requestAnimationFrame(frame);
  }
  function still(){reveal=1;lastAmb=performance.now()-900;draw(performance.now(),1.3)}
  function go(){ if(reduce){still();return} if(!raf&&vis) raf=requestAnimationFrame(frame) }
  var rt;
  window.addEventListener('resize',function(){clearTimeout(rt);rt=setTimeout(function(){build();if(reduce)still()},150)});
  hero.addEventListener('pointermove',function(e){var b=hero.getBoundingClientRect();ptr.x=e.clientX-b.left;ptr.y=e.clientY-b.top},{passive:true});
  hero.addEventListener('pointerleave',function(){ptr.x=ptr.y=-999});
  hero.addEventListener('pointerdown',function(e){if(e.target.closest('a,button,.hwin'))return;var b=hero.getBoundingClientRect();ripple(e.clientX-b.left,e.clientY-b.top,1.1)});
  if('IntersectionObserver' in window){new IntersectionObserver(function(es){vis=es[0].isIntersecting&&!document.hidden; if(vis&&!raf&&!reduce&&ready) raf=requestAnimationFrame(frame)}).observe(hero)}
  document.addEventListener('visibilitychange',function(){ if(document.hidden){vis=false}else{vis=true;if(!raf&&!reduce&&ready)raf=requestAnimationFrame(frame)} });
  var started=false; function init(){if(started)return;started=true;build();go()}
  if(document.fonts&&document.fonts.load){Promise.race([document.fonts.load('12px "Geist Mono Variable"'),new Promise(function(r){setTimeout(r,1800)})]).then(init,init)}else init();
  window.addEventListener('load',function(){if(started){build();if(reduce)still()}});
  if(document.fonts&&document.fonts.ready){document.fonts.ready.then(function(){if(started){build();if(reduce)still()}})}
  Field={
    attach:function(on){attached=on;if(amber)ripple(amber.cx,amber.cy,1.4);if(reduce&&ready)still()},
    spawn:function(){
      var x=ptr.x>0?ptr.x:W*(.15+Math.random()*.7), y=ptr.y>0?ptr.y:H*(.1+Math.random()*.5), best=null,bd=1e9;
      tiles.forEach(function(T){if(T.isAmber||T.k<.85)return;var d=Math.hypot(T.cx-x,T.cy-y);if(d<bd){bd=d;best=T}});
      if(best){best.pop=performance.now();best.working=true;best.pet=true;ripple(best.cx,best.cy,1.3);if(reduce)still()}
      return !!best;
    },
    ripple:ripple, onHover:null
  };
  return Field;
}
