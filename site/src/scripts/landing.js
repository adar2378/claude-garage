import { doc, $, $$, reduce, mobileMQ, clamp } from './util.js';
import { createField } from './field.js';

var lenis=null, st=null, current=0;

/* ---------- toast + keycap flash ---------- */
var toastT;
function toast(msg){var t=$('#toast');t.textContent=msg;t.classList.add('show');clearTimeout(toastT);toastT=setTimeout(function(){t.classList.remove('show')},2000)}
function flash(k){$$('.kcap[data-k="'+k+'"]').forEach(function(el){el.classList.add('hit');setTimeout(function(){el.classList.remove('hit')},260)})}

/* ---------- pit pet ---------- */
var pet=$('#pet'), pg=$('#pg'), ps=$('#ps'), petState='asleep', petLock=null, petIdle=0, petSay='zzz', ctxSay='psst. press a.', pointer={x:null,y:0}, lastWake=0;
function glyph(){
  if(petState==='asleep') return '=-.-=';
  if(petState==='alert') return '(0.0) !';
  if(petState==='happy') return '(^.^)';
  if(pointer.x===null) return '(0.0)';
  var r=pet.getBoundingClientRect(), dx=pointer.x-(r.left+r.width/2);
  return dx>90?'(>.>)':dx<-90?'(<.<)':'(0.0)';
}
function paintPet(){
  pet.className='pet'+(petState==='alert'?' alert awake':petState==='asleep'?'':' awake')+(petAway?' away':'');
  var g=glyph(); pg.textContent=g; ps.textContent=petSay;
  var lg=petState==='asleep'?'(0.0)':g;
  $$('[data-logo]').forEach(function(el){el.textContent=lg});
}
var petAway=false;
function setPet(state,say){ if(petLock&&state!=='alert'&&state!=='happy'){state=petLock.state;say=petLock.say} petState=state; if(say) petSay=say; paintPet(); }
function wake(){
  if(petState==='asleep') setPet('awake',ctxSay);
  clearTimeout(petIdle);
  petIdle=setTimeout(function(){ if(!petLock&&petState!=='asleep'){petSay='zzz';setPet('asleep','zzz')} },2600);
}
function lock(on,say){ petLock=on?{state:'alert',say:say}:null; if(on){petState='alert';petSay=say}else{petState='awake';petSay=ctxSay} paintPet(); if(!on) wake(); else {clearTimeout(petIdle)} }
var alerts={};
function setAlert(src,on){var was=alerts.a||alerts.h;alerts[src]=on;var now=alerts.a||alerts.h;if(now&&!was)lock(true,'someone needs you');else if(!now&&was)lock(false)}
function happy(say){setPet('happy',say);setTimeout(function(){setPet('awake',say);wake()},900)}
window.addEventListener('pointermove',function(e){
  pointer.x=e.clientX;pointer.y=e.clientY;
  if(petState==='awake') pg.textContent=glyph();
  var n=performance.now(); if(n-lastWake>140){lastWake=n;wake()}
},{passive:true});
pet.addEventListener('click',function(){var l=['still here.','scroll, I will follow.','amber means you are needed.','tmux has the sessions.','try pressing a.'];var s=l[Math.floor(Math.random()*l.length)];petSay=s;happy(s)});
pet.addEventListener('keydown',function(e){if(e.key==='Enter'||e.key===' '){e.preventDefault();pet.click()}});
paintPet();
var lastY=window.scrollY;
window.addEventListener('scroll',function(){ if(Math.abs(window.scrollY-lastY)>2){wake()} lastY=window.scrollY; },{passive:true});

/* section lines + away zones */
var stepSay=['press n for a new one.','everyone in one place.','someone needs you.','press a. I will wait.','tmux has them.'];
var sectionSay={hero:'psst. press a.',story:null,plugins:'I am one of the nine.',keys:'try pressing a key.',install:'take me with you.',closing:'go build.'};
var inStoryZone=false;
if('IntersectionObserver' in window){
  var mo=new IntersectionObserver(function(es){es.forEach(function(en){
    if(!en.isIntersecting) return;
    var id=en.target.id; inStoryZone=(id==='story');
    ctxSay=inStoryZone?stepSay[current]:(sectionSay[id]||'zzz');
    if(!petLock){petSay=ctxSay; if(petState!=='asleep')paintPet()}
  })},{rootMargin:'-45% 0px -50% 0px'});
  ['hero','story','plugins','keys','install','closing'].forEach(function(id){var el=document.getElementById(id);if(el)mo.observe(el)});
  var seen={};
  var ao=new IntersectionObserver(function(es){es.forEach(function(en){seen[en.target.id||en.target.tagName]=en.isIntersecting});updAway()},{threshold:0});
  var hb=$('#hintbar');
  function updAway(){
    var away=!!seen.FOOTER || (mobileMQ.matches&&(!!seen.install||!!seen.closing));
    if(away!==petAway){petAway=away;pet.classList.toggle('away',away)}
    hb.classList.toggle('away',!!seen.FOOTER);
  }
  ['install','closing'].forEach(function(id){ao.observe(document.getElementById(id))});
  ao.observe($('footer'));
  mobileMQ.addEventListener&&mobileMQ.addEventListener('change',updAway);
}

/* ---------- copy buttons ---------- */
document.addEventListener('click',function(e){
  var b=e.target.closest('[data-copy]'); if(!b) return;
  var t=b.getAttribute('data-copy');
  var done=function(){
    var l=b.querySelector('.lbl'); b.classList.add('done');
    if(l){l.textContent='Copied'}else{b.dataset.o=b.textContent;b.textContent='Copied'}
    setTimeout(function(){b.classList.remove('done'); if(l){l.textContent='Copy'}else{b.textContent=b.dataset.o||'Copy'}},1600);
    toast('copied to clipboard');
  };
  if(navigator.clipboard&&navigator.clipboard.writeText){navigator.clipboard.writeText(t).then(done,done)}
  else{var ta=document.createElement('textarea');ta.value=t;document.body.appendChild(ta);ta.select();try{document.execCommand('copy')}catch(_){ }ta.remove();done()}
});

/* ---------- install tabs ---------- */
var tabs=$$('.tab');
function selTab(i,focus){tabs.forEach(function(t,k){var on=k===i;t.setAttribute('aria-selected',on);t.tabIndex=on?0:-1;document.getElementById(t.getAttribute('aria-controls')).hidden=!on;if(on&&focus)t.focus()})}
tabs.forEach(function(t,i){t.addEventListener('click',function(){selTab(i)});t.addEventListener('keydown',function(e){if(e.key==='ArrowRight')selTab((i+1)%tabs.length,1);if(e.key==='ArrowLeft')selTab((i+tabs.length-1)%tabs.length,1)})});

/* ---------- reveals ---------- */
var rv=$$('.rv');
if(!reduce&&'IntersectionObserver' in window){
  doc.classList.add('rv-on');
  var io=new IntersectionObserver(function(es){es.forEach(function(e){if(e.isIntersecting){e.target.classList.add('in');io.unobserve(e.target)}})},{rootMargin:'0px 0px -8% 0px',threshold:.08});
  rv.forEach(function(el){io.observe(el)});
}


/* ---------- hero: ASCII tile field on canvas ---------- */
var Field=createField();
Field.onHover=function(h){ if(heroAttached) return; setAlert('h',h) };

/* ---------- hero: swap to the engaged wall ---------- */
var heroAttached=false;
function heroAttach(on){
  heroAttached=on;
  var w=$('#hwin'); w.classList.toggle('att',on);
  var b=$('#hattach'); b.setAttribute('aria-pressed',on); $('#hlbl').textContent=on?'attached':'needs you';
  $('#hwt').textContent=on?'web / claude-1':'claude-garage';
  Field.attach(on);
  if(on){ setAlert('h',false); setAlert('a',false); toast('a → attached to web/claude-1. Esc to go back.'); petState='happy'; petSay='attached. esc to detach.'; paintPet(); setTimeout(function(){ if(heroAttached){setPet('awake','attached. esc to detach.');wake()} },900) }
  else { toast('Esc → back to the wall'); wake() }
}
$('#hattach').addEventListener('click',function(){heroAttach(!heroAttached)});

/* ---------- story ---------- */
var steps=$$('.step');
var btns=$$('.sbtn');
var notch=$('.notch');
var progBar=$('.prog i');
function setActive(i){
  if(i===current&&steps[i].classList.contains('on')) return;
  current=i;
  steps.forEach(function(s,k){s.classList.toggle('on',k===i)});
  btns.forEach(function(b,k){var on=k===i;b.setAttribute('aria-selected',on);b.tabIndex=on?0:-1});
  notch.style.setProperty('--i',i);
  onStep(i);
}
function onStep(i){
  setAlert('a',i===2&&inStoryZone);
  if(inStoryZone){ctxSay=stepSay[i]; if(!petLock){petSay=ctxSay;paintPet()} wake()}
}
btns.forEach(function(b,i){
  b.addEventListener('click',function(){goStep(i)});
  b.addEventListener('keydown',function(e){var n=null;if(e.key==='ArrowRight')n=Math.min(4,current+1);if(e.key==='ArrowLeft')n=Math.max(0,current-1);if(n!==null){e.preventDefault();goStep(n);btns[n].focus()}});
});
function yFor(i){return st.start+((i+.5)/5)*(st.end-st.start)}
function goTo(sel){
  var el=sel==='#hero'?null:$(sel);
  if(lenis){ if(el)lenis.scrollTo(el,{offset:-10,duration:1.2}); else lenis.scrollTo(0,{duration:1.2}) }
  else { if(el)el.scrollIntoView({behavior:reduce?'auto':'smooth',block:'start'}); else window.scrollTo({top:0,behavior:reduce?'auto':'smooth'}) }
}
function goStep(i){
  if(st){
    var y=yFor(i);
    if(lenis){lenis.scrollTo(y,{duration:1.2})}else{window.scrollTo({top:y,behavior:reduce?'auto':'smooth'})}
  }else{
    var el=document.getElementById('s'+i); if(el) el.scrollIntoView({behavior:reduce?'auto':'smooth',block:'center'});
  }
}
/* stacked mode (no pin): follow the steps with an observer */
if('IntersectionObserver' in window){
  var so=new IntersectionObserver(function(es){es.forEach(function(en){
    if(st||!en.isIntersecting) return;
    var i=+en.target.getAttribute('data-i'); current=i; inStoryZone=true;
    setAlert('a',i===2); ctxSay=stepSay[i]; if(!petLock){petSay=ctxSay;paintPet()} wake();
  })},{rootMargin:'-42% 0px -42% 0px'});
  steps.forEach(function(s){so.observe(s)});
}

/* replay "nothing gets lost" */
var replayBusy=false;
function replayLost(){
  if(replayBusy) return; replayBusy=true;
  var s=steps[4], lab=$('#rest'), dot=lab.previousElementSibling, caps=$$('.key.w',s);
  lab.textContent='restoring 3 sessions...'; dot.style.opacity='.35';
  caps.forEach(function(c,k){setTimeout(function(){c.classList.add('hit')},k*260);setTimeout(function(){c.classList.remove('hit')},k*260+220)});
  if(st&&!reduce){s.classList.remove('on');setTimeout(function(){s.classList.add('on')},320)}
  setTimeout(function(){lab.textContent='restored 3 sessions';dot.style.opacity='';happy('resumed.');toast('r r → restarted and resumed the conversation');replayBusy=false},1300);
}

/* ---------- jump palette ---------- */
var pal=$('#pal'), pq=$('#pq'), pl=$('#pl'), palOpen=false, psel=0, pitems=[], palFocus=null;
var ACTIONS=[
  {t:'Top',s:'hero',go:'#hero'},{t:'How it works',s:'section',go:'#story'},{t:'Plugins',s:'section',go:'#plugins'},{t:'Keys',s:'section',go:'#keys'},{t:'Install',s:'section',go:'#install'},
  {t:'Step 1: Start',s:'story',step:0},{t:'Step 2: See them all',s:'story',step:1},{t:'Step 3: Spot the amber one',s:'story',step:2},{t:'Step 4: Press a and answer',s:'story',step:3},{t:'Step 5: Nothing gets lost',s:'story',step:4},
  {t:'Copy npx claude-garage',s:'clipboard',fn:function(){var b=$('[data-copy]');b.click()}},
  {t:'Open GitHub',s:'new tab',fn:function(){window.open('https://github.com/adar2378/claude-garage','_blank','noopener')}}
];
function esc(s){return s.replace(/[&<>]/g,function(c){return {'&':'&amp;','<':'&lt;','>':'&gt;'}[c]})}
function drawPal(){
  var v=pq.value.trim().toLowerCase();
  pitems=ACTIONS.filter(function(a){return !v||a.t.toLowerCase().indexOf(v)>-1||a.s.indexOf(v)>-1});
  psel=clamp(psel,0,Math.max(0,pitems.length-1));
  pl.innerHTML=pitems.map(function(a,i){return '<li role="option" aria-selected="'+(i===psel)+'" class="'+(i===psel?'on':'')+'" data-i="'+i+'">'+esc(a.t)+'<small>'+esc(a.s)+'</small></li>'}).join('')||'<li><small>nothing matches. try "plugins".</small></li>';
  var on=$('li.on',pl); if(on&&on.scrollIntoView) on.scrollIntoView({block:'nearest'});
}
function openPal(){ if(palOpen) return; palOpen=true; palFocus=document.activeElement; pal.hidden=false; pq.value=''; psel=0; drawPal(); if(lenis) lenis.stop(); setTimeout(function(){pq.focus()},10); }
function closePal(){ if(!palOpen) return; palOpen=false; pal.hidden=true; if(lenis) lenis.start(); if(palFocus&&palFocus.focus)try{palFocus.focus({preventScroll:true})}catch(_){ } }
function runPal(i){ var a=pitems[i]; if(!a) return; closePal(); setTimeout(function(){ if(a.go) goTo(a.go); else if(a.step!==undefined) goStep(a.step); else a.fn() },40); }
pq.addEventListener('input',function(){psel=0;drawPal()});
pq.addEventListener('keydown',function(e){
  if(e.key==='ArrowDown'){e.preventDefault();psel=Math.min(psel+1,pitems.length-1);drawPal()}
  else if(e.key==='ArrowUp'){e.preventDefault();psel=Math.max(psel-1,0);drawPal()}
  else if(e.key==='Enter'){e.preventDefault();runPal(psel)}
  else if(e.key==='Escape'){e.preventDefault();closePal()}
  else if(e.key==='Tab'){e.preventDefault()}
});
pl.addEventListener('click',function(e){var li=e.target.closest('li[data-i]');if(li)runPal(+li.getAttribute('data-i'))});
pal.addEventListener('pointerdown',function(e){if(e.target===pal)closePal()});

/* ---------- global keys ---------- */
var rPrimed=0;
function heroInView(){var r=$('#hero').getBoundingClientRect();return r.bottom>260&&r.top<window.innerHeight*.6}
document.addEventListener('keydown',function(e){
  if(e.metaKey||e.ctrlKey||e.altKey||e.isComposing) return;
  var tg=e.target; if(tg&&(tg.tagName==='INPUT'||tg.tagName==='TEXTAREA'||tg.tagName==='SELECT'||tg.isContentEditable)) return;
  var k=e.key, now=Date.now();
  if(palOpen){ if(k==='Escape'){e.preventDefault();closePal()} return }
  if(e.repeat) return;
  if(k==='Escape'){ if(heroAttached){heroAttach(false)} return }
  if(k==='/'){e.preventDefault();wake();flash('slash');openPal();return}
  if(k==='a'||k==='A'){
    wake(); flash('a');
    if(heroInView()) heroAttach(!heroAttached);
    else { goStep(3); toast('a → jumping to the session that needs you') }
    return;
  }
  if(k==='n'||k==='N'){
    wake(); flash('n');
    var worktree=(k==='N');
    var doIt=function(){Field.spawn();toast(worktree?'N → new session in a worktree':'n → new session');happy('a new tile!')};
    if(heroInView()) doIt(); else { goTo('#hero'); setTimeout(doIt,reduce?50:1100) }
    return;
  }
  if(k==='r'||k==='R'){
    wake();
    if(now-rPrimed<900){
      rPrimed=0; flash('r2');
      var r=steps[4].getBoundingClientRect();
      var visible=st?(current===4&&inStoryZone):(r.bottom>0&&r.top<window.innerHeight);
      if(visible) replayLost(); else { goStep(4); setTimeout(replayLost,reduce?100:1500) }
    } else { rPrimed=now; flash('r'); toast('r ... once more for r r') }
    return;
  }
});


/* ---------- motion: GSAP + Lenis, loaded lazily, never under prefers-reduced-motion ---------- */
function boot(gsap,ScrollTrigger,Lenis){
  gsap.registerPlugin(ScrollTrigger);
  try{
    lenis=new Lenis({lerp:.1,wheelMultiplier:1,smoothWheel:true});
    lenis.on('scroll',ScrollTrigger.update);
    gsap.ticker.add(function(t){lenis.raf(t*1000)});
    gsap.ticker.lagSmoothing(0);
    $$('a[href^="#"]').forEach(function(a){a.addEventListener('click',function(e){var id=a.getAttribute('href');if(id.length<2)return;var el=document.querySelector(id);if(!el)return;e.preventDefault();lenis.scrollTo(id==='#top'?0:el,{offset:-10,duration:1.2})})});
  }catch(_){lenis=null}
  var mm=gsap.matchMedia();
  mm.add('(min-width: 900px) and (min-height: 620px) and (prefers-reduced-motion: no-preference)',function(){
    doc.classList.add('pin');
    st=ScrollTrigger.create({
      trigger:'.story',start:'top top',end:function(){return '+='+Math.round(window.innerHeight*4.4)},
      pin:'.stage-pin',pinSpacing:true,anticipatePin:1,invalidateOnRefresh:true,
      onToggle:function(self){inStoryZone=self.isActive||inStoryZone; },
      onEnter:function(){inStoryZone=true;onStep(current)},onEnterBack:function(){inStoryZone=true;onStep(current)},
      onLeave:function(){setAlert('a',false)},onLeaveBack:function(){setAlert('a',false)},
      onUpdate:function(self){
        var p=self.progress;
        setActive(Math.max(0,Math.min(4,Math.floor(p*5))));
        if(progBar)progBar.style.transform='scaleX('+p+')';
      }
    });
    setActive(0);
    return function(){doc.classList.remove('pin');st=null;steps.forEach(function(s){s.classList.remove('on')});steps[0].classList.add('on');if(progBar)progBar.style.transform=''};
  });
}
/* Phones and short windows keep native scrolling: the libraries are fetched only where the pinned story can run. */
var motionMQ=window.matchMedia('(min-width: 900px) and (min-height: 620px)');
var motionLoaded=false;
function loadMotion(){
  if(motionLoaded||reduce||!motionMQ.matches) return;
  motionLoaded=true;
  Promise.all([import('gsap'),import('gsap/ScrollTrigger'),import('lenis')]).then(function(m){
    boot(m[0].gsap||m[0].default,m[1].ScrollTrigger||m[1].default,m[2].default);
  }).catch(function(){ /* the page works without the motion libraries */ });
}
loadMotion();
if(motionMQ.addEventListener) motionMQ.addEventListener('change',loadMotion);
