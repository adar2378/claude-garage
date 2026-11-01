import { $, clamp, reduce } from './util.js';

/* Hero: ASCII tile field on canvas. */
export function createField() {
  var Field;
  var cv=$('#field'), ctx=cv.getContext('2d'), hero=$('#hero');
  /* The amber tile lives in its own small canvas, drawn once. Its blink and pulse ring are CSS animations (compositor only),
     so the main canvas only redraws when something changes: the reveal, a ripple, a popped tile or the pointer. */
  var wrap=document.createElement('div'), amb=document.createElement('canvas'), ring=document.createElement('i'), actx=amb.getContext('2d');
  wrap.className='amb'; wrap.setAttribute('aria-hidden','true'); ring.className='amb-ring';
  wrap.appendChild(amb); wrap.appendChild(ring); cv.after(wrap);
  var AM=26, until=0, lastDraw=0;
  var W=0,H=0,dpr=1,cw=7.2,ch=17,fs=12,tiles=[],amber=null,t0=0,vis=true,raf=0,attached=false,ptr={x:-999,y:-999},rips=[],hover=false,reveal=reduce?1:0,rects=[],ready=false;
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
    /* The amber tile sits in the right gutter beside the terminal, clear of the headline. Narrow screens have no free gutter, so they get none. */
    amber=null; var hw=$('#hwin');
    if(W>=1240&&hw){
      var hr=rel(hw), tx=W-(W-hr.w)/4, ty=hr.y+60, best=1e9;
      tiles.forEach(function(T){if(T.md<24||T.y+T.h>H-4)return;var d=Math.hypot(T.cx-tx,T.cy-ty);if(d<best&&T.x>-cw&&T.x+T.w<W+cw){best=d;amber=T}});
    }
    if(amber){amber.k=1;amber.isAmber=true}
    wrap.style.display=amber?'':'none';
    paintAmber();
    ready=true;
  }
  function ripple(x,y,s){rips.push({x:x,y:y,t:performance.now(),s:s||1});if(rips.length>8)rips.shift();kick(2700)}
  function kick(ms){var n=performance.now();until=Math.max(until,n+(ms||0));if(small()&&n>=until)return;if(!raf&&vis&&ready&&!reduce)raf=requestAnimationFrame(frame)}
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
  function paintAmber(){
    if(!amber) return;
    var T=amber, w=Math.ceil(T.w+2*AM), h=Math.ceil(T.h+2*AM+14), x0=AM, y0=AM;
    amb.width=Math.round(w*dpr);amb.height=Math.round(h*dpr);
    amb.style.cssText='width:'+w+'px;height:'+h+'px';
    wrap.style.left=(T.x-AM)+'px';wrap.style.top=(T.y-AM)+'px';wrap.style.width=w+'px';wrap.style.height=h+'px';
    var c=actx, col=attached?'232,80,2':'255,176,32', S=SETS[1], m=S[1].repeat(TW-2), font='400 '+fs+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';
    c.setTransform(dpr,0,0,dpr,0,0);c.clearRect(0,0,w,h);c.font=font;c.textBaseline='top';
    c.save();
    c.fillStyle='rgba('+col+','+(attached?.1:.2)+')';
    c.fillRect(x0-cw*.2,y0-ch*.1,T.w+cw*.4,T.h+ch*.1);
    c.shadowColor='rgba('+col+',.8)';c.shadowBlur=attached?8:16;
    c.fillStyle='rgba('+col+',1)';
    c.fillText(S[0]+m+S[2],x0,y0);
    c.fillText(S[4]+m+S[5],x0,y0+ch*4);
    c.fillText(S[3]+'        '+S[3],x0,y0+ch);c.fillText(S[3]+'        '+S[3],x0,y0+ch*2);c.fillText(S[3]+'        '+S[3],x0,y0+ch*3);
    c.restore();
    c.fillStyle=attached?'rgba(179,62,0,.95)':'rgba(107,67,0,.95)';
    var inner=attached?['  ▰▰▰▱  ',' (^.^)  ','  ▪▪▪   ']:['  ▰▰▰▱  ',' (0.0) !','  ▪▪▪   '];
    for(var i=0;i<3;i++){c.fillText(inner[i],x0+cw,y0+ch*(i+1))}
    c.font='500 '+(fs-1)+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';
    var lb=attached?'attached':'needs you';
    c.fillText(lb,x0+T.w/2-c.measureText(lb).width/2,y0+T.h+8);
    ring.style.left=(x0-7.2)+'px';ring.style.top=(y0-4)+'px';ring.style.width=(T.w+14.4)+'px';ring.style.height=(T.h+8)+'px';
    ring.style.setProperty('--sx',(T.w+97.2)/(T.w+14.4));ring.style.setProperty('--sy',(T.h+54)/(T.h+8));
    wrap.classList.toggle('on',!attached);
  }
  function draw(now,t,from,to){
    from=from||0; to=to===undefined?tiles.length:to;
    if(!from) ctx.clearRect(0,0,W,H);
    if(amber){var da=Math.hypot(amber.cx-W/2,amber.cy-H*.3)/Math.hypot(W/2,H*.7);wrap.style.opacity=clamp(reveal*2-da,0,1)}
    ctx.font='400 '+fs+'px "Geist Mono Variable",ui-monospace,Menlo,monospace';ctx.textBaseline='top';
    for(var i=from;i<to;i++){
      var T=tiles[i]; if(T.isAmber) continue; if(T.x>W||T.x+T.w<0||T.y>H||T.y+T.h<0) continue;
      var popping=(now-T.pop)<2400;
      if(T.k<.02&&!popping&&!T.isAmber) continue;
      var dn=Math.hypot(T.cx-W/2,T.cy-H*.3)/Math.hypot(W/2,H*.7);
      var ra=clamp(reveal*2-dn,0,1); if(ra<=0) continue;
      ctx.globalAlpha=ra;
      drawTile(T,intensity(T,now),now);
    }
    ctx.globalAlpha=1;
  }
  function frame(now){
    raf=0; if(!vis||!ready) return;
    if(lastDraw&&now-lastDraw<32){ /* at most ~30 fps: wait out the rest of the slot instead of polling every frame */
      raf=-1; setTimeout(function(){raf=0;if(vis)raf=requestAnimationFrame(frame)},Math.max(1,34-(now-lastDraw))); return;
    }
    lastDraw=now;
    if(!t0) t0=now;
    var t=(now-t0)/1000;
    reveal=small()?1:sm((t-.3)/2.2);
    if(t<.05&&!rips.length&&!small()) ripple(W*.5,H*.3,1.1);
    draw(now,t);
    /* keep going only while something is still moving */
    if((reveal<1||now<until)&&!raf) raf=requestAnimationFrame(frame);
  }
  function hoverCheck(){
    if(!amber) return;
    var ah=Math.abs(ptr.x-(amber.cx))<amber.w/2+14&&Math.abs(ptr.y-amber.cy)<amber.h/2+14;
    if(ah!==hover){hover=ah;Field.onHover&&Field.onHover(ah)}
  }
  function still(){reveal=1;draw(performance.now(),1.3)}
  /* Phones skip the reveal animation: one static paint, drawn in small slices so no single task blocks input. */
  var idle=window.requestIdleCallback||function(f){return setTimeout(f,16)};
  function stillChunked(){
    reveal=1; var i=0, now=performance.now();
    (function step(){ var end=Math.min(i+12,tiles.length); draw(now,1.3,i,end); i=end; if(i<tiles.length) idle(step); })();
  }
  function small(){return W<700}
  function redraw(){ if(reduce) still(); else if(small()) stillChunked(); else kick(0) }
  function go(){ if(reduce){still();return} if(small()){stillChunked();return} kick(0) }
  var rt;
  window.addEventListener('resize',function(){clearTimeout(rt);rt=setTimeout(function(){build();redraw()},150)});
  hero.addEventListener('pointermove',function(e){var b=hero.getBoundingClientRect();ptr.x=e.clientX-b.left;ptr.y=e.clientY-b.top;hoverCheck();kick(0)},{passive:true});
  hero.addEventListener('pointerleave',function(){ptr.x=ptr.y=-999;hoverCheck();kick(0)});
  hero.addEventListener('pointerdown',function(e){if(e.target.closest('a,button,.hwin'))return;var b=hero.getBoundingClientRect();ripple(e.clientX-b.left,e.clientY-b.top,1.1)});
  if('IntersectionObserver' in window){new IntersectionObserver(function(es){vis=es[0].isIntersecting&&!document.hidden; if(vis) kick(0)}).observe(hero)}
  document.addEventListener('visibilitychange',function(){ vis=!document.hidden; if(vis) kick(0) });
  /* a faint ripple now and then, only while the hero is on screen */
  (function ambient(){setTimeout(function(){ if(vis&&!reduce&&!small()&&ready&&reveal>=1) ripple(W*(.12+Math.random()*.76),H*(.08+Math.random()*.6),.8); ambient() },9000)})();
  /* One build, after the mono font and its box-drawing subset are in (or after 1.8 s). */
  var started=false; function init(){if(started)return;started=true;idle(function(){build();go()},{timeout:1500})}
  if(document.fonts&&document.fonts.load){
    Promise.race([Promise.all([document.fonts.load('12px "Geist Mono Variable"','┌┏╔─│'),document.fonts.ready]),new Promise(function(r){setTimeout(r,1800)})]).then(init,init);
  }else init();
  Field={
    attach:function(on){attached=on;paintAmber();if(amber)ripple(amber.cx,amber.cy,1.4);if(reduce&&ready)still()},
    spawn:function(){
      var x=ptr.x>0?ptr.x:W*(.15+Math.random()*.7), y=ptr.y>0?ptr.y:H*(.1+Math.random()*.5), best=null,bd=1e9;
      tiles.forEach(function(T){if(T.isAmber||T.k<.85)return;var d=Math.hypot(T.cx-x,T.cy-y);if(d<bd){bd=d;best=T}});
      if(best){best.pop=performance.now();best.working=true;best.pet=true;ripple(best.cx,best.cy,1.3);kick(2500);if(reduce)still()}
      return !!best;
    },
    ripple:ripple, onHover:null
  };
  return Field;
}
