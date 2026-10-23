'use strict';
const $=s=>document.querySelector(s), $$=s=>[...document.querySelectorAll(s)];
const access=new URLSearchParams(location.hash.slice(1)).get('access');
if(access){sessionStorage.setItem('kern-key',access);history.replaceState(null,'',location.pathname);}
const key=sessionStorage.getItem('kern-key')||'';
let map={parameters:[]}, revision=0, activeRevision=0, snapshot={sources:[],parameters:[]}, tab='charts', plots=[], tracks=[], dirty=false, busy=false, offset=0, mode=0, firstTime=Date.now(), lastDraw=0, sizeDirty=true;
let autoScale=true;
let flags=JSON.parse(localStorage.getItem('kern-flags')||'[]').slice(-80), editing=-1, pickerTrack=0;
const fmt=v=>v==null?'—':Number(v).toLocaleString('ru-RU',{maximumFractionDigits:2});
const time=t=>new Date(t).toLocaleTimeString('ru-RU',{hour12:false});
function note(text){$('#message').textContent=text;}
async function api(path,body){
 const options={headers:{'X-Kern-Key':key}}; if(body!==undefined){options.method='POST';options.headers['Content-Type']='application/json';options.body=JSON.stringify(body);}
 for(let n=0;n<20;n++){const r=await fetch('/api/'+path,options);if(!r.ok)throw Error(await r.text());const v=await r.json();if(v.result==='error'){if(v.code==='busy'){await new Promise(r=>setTimeout(r,80));continue;}throw Error(v.message);}return v;}throw Error('Ядро занято');
}
function element(tag,cls,text){const e=document.createElement(tag);if(cls)e.className=cls;if(text!==undefined)e.textContent=text;return e;}
function current(id){return snapshot.parameters.find(p=>p.id===id);}
function definition(id){return map.parameters.find(p=>p.id===id);}
function layout(){
 const allowed=new Set(map.parameters.map(p=>p.id));tracks=tracks.map(t=>t.filter(id=>allowed.has(id))).filter(t=>t.length);
 if(!tracks.length)tracks=map.parameters.slice(0,6).map(p=>[p.id]);
 const present=new Set(tracks.flat());for(const p of map.parameters.slice(0,6))if(!present.has(p.id)&&tracks.length<6)tracks.push([p.id]);
 for(const p of plots)p.u.destroy();plots=[];$('#tracks').replaceChildren();
 const headerHeight=62*Math.max(1,...tracks.map(t=>t.length));$('#history').style.top=headerHeight+'px';$('#history').style.height=`calc(100% - ${headerHeight+18}px)`;
 tracks.forEach((ids,index)=>{
  const track=element('div','track');const header=element('div','trackHeader');header.style.height=headerHeight+'px';
  for(const id of ids){const p=definition(id),row=element('div');row.style.color=p.color;row.style.height='62px';const title=element('div','title');title.append(element('span','',p.name+' · '+p.unit),element('b','',fmt(current(id)?.value)));row.append(title);const scale=element('div','scale');for(let n=0;n<=4;n++)scale.append(element('span','',fmt(p.min+(p.max-p.min)*n/4)));row.append(scale);header.append(row);row.addEventListener('click',()=>edit(map.parameters.findIndex(p=>p.id===id)));}
  const area=element('div','plot'),pen=element('div','pen'),split=element('div','splitter');track.append(header,area,pen,split);$('#tracks').append(track);
  const p0=definition(ids[0]);const scales={x:{time:false,ori:1,dir:-1,auto:false}};for(const id of ids){const p=definition(id);scales[id]={ori:0,dir:1,auto:false,range:[p.min,p.max]};}
  const item={track,header,area,pen,ids,from:0,to:1,points:{},flags:[],tooltip:null,cursor:[]};
  const u=new uPlot({width:Math.max(100,area.clientWidth),height:Math.max(100,area.clientHeight),padding:[0,0,0,0],scales,series:[{},...ids.map(id=>({label:id,scale:id,stroke:definition(id).color,width:1.5,spanGaps:false,points:{show:false}}))],axes:[{show:false},...ids.map(id=>({scale:id,show:false}))],legend:{show:false},cursor:{show:false},select:{show:false},hooks:{drawClear:[u=>tileGrid(item,u)],draw:[u=>{item.u=u;placeFlags(item);placePens(item);}] }},[[],...ids.map(()=>[])],area);item.u=u;plots.push(item);
  area.addEventListener('dblclick',e=>flagAt(item,e));area.addEventListener('contextmenu',e=>{e.preventDefault();pickerTrack=index;const select=$('#curveChoice');select.replaceChildren();for(const p of map.parameters.filter(p=>!ids.includes(p.id)))select.append(new Option(p.name+' · '+p.unit,p.id));$('#curvePicker').showModal();});
  area.addEventListener('mousemove',e=>cursorAt(item,e));area.addEventListener('mouseleave',()=>clearCursor(item));area.addEventListener('wheel',e=>{e.preventDefault();offset=Math.max(0,Math.min(86400-Number($('#interval').value),offset+Math.sign(e.deltaY)*Number($('#interval').value)/8));sizeDirty=true;}, {passive:false});
  split.addEventListener('pointerdown',e=>{const next=track.nextElementSibling;if(!next)return;split.setPointerCapture(e.pointerId);const x=e.clientX,w=track.getBoundingClientRect().width,total=w+next.getBoundingClientRect().width;track.style.flex='none';next.style.flex='none';split.classList.add('drag');split.onpointermove=ev=>{const width=Math.max(100,Math.min(total-100,w+ev.clientX-x));track.style.width=width+'px';next.style.width=(total-width)+'px';sizeDirty=true;};split.onpointerup=()=>{split.onpointermove=null;split.classList.remove('drag');};});
 });
 sizeDirty=true;
}
function tileGrid(item,u){
 const ctx=u.ctx,b=u.bbox,ratio=uPlot.pxRatio;ctx.save();ctx.beginPath();ctx.rect(b.left,b.top,b.width,b.height);ctx.clip();
 ctx.fillStyle='#1d2021';ctx.fillRect(b.left,b.top,b.width,b.height);
 const height=b.height/ratio,range=item.to-item.from,step=gridStep(range,height),minor=step/4,col=b.width/8;
 for(let t=Math.floor(item.from/minor)*minor;t<=item.to;t+=minor){const y=u.valToPos(t,'x',true),y2=u.valToPos(t+minor,'x',true);for(let n=0;n<8;n++){ctx.fillStyle=((Math.round(t/minor)+n)&1)?'#ffffff04':'#00000005';ctx.fillRect(b.left+n*col,Math.min(y,y2),col,Math.abs(y2-y));}}
 ctx.strokeStyle='#a899842b';ctx.lineWidth=ratio*.7;ctx.beginPath();for(let n=0;n<=4;n++){const x=b.left+n*b.width/4;ctx.moveTo(x,b.top);ctx.lineTo(x,b.top+b.height);}for(let t=Math.ceil(item.from/step)*step;t<=item.to;t+=step){const y=u.valToPos(t,'x',true);ctx.moveTo(b.left,y);ctx.lineTo(b.left+b.width,y);}ctx.stroke();ctx.restore();
}
function gridStep(range,height){return [1000,2000,5000,10000,15000,30000,60000,120000,300000,600000,1800000,3600000,7200000].find(s=>s/range*height>=70)||14400000;}
function placePens(item){
 item.pen.replaceChildren();if(offset>0)return;item.ids.forEach((id,i)=>{const sample=current(id),p=definition(id);if(!sample||sample.value==null)return;const x=item.u.valToPos(sample.value,id);if(x<0||x>item.area.clientWidth)return;
 const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.style.left=x+'px';const path=document.createElementNS(svg.namespaceURI,'path');const inset=i*2;path.setAttribute('d',`M ${inset+1} ${14-inset} L 8 ${1+inset} L ${15-inset} ${14-inset} Z`);path.setAttribute('fill','none');path.setAttribute('stroke',p.color);path.setAttribute('stroke-width','1.5');svg.append(path);item.pen.append(svg);});
}
function clearCursor(item){for(const e of item.cursor)e.remove();item.cursor=[];}
function cursorAt(item,e){
 clearCursor(item);const rect=item.area.getBoundingClientRect(),y=e.clientY-rect.top;const stamp=item.u.posToVal(y,'x');
 const line=element('div','crosshair');line.style.top=y+'px';item.area.append(line);item.cursor.push(line);
 let offsetY=0;for(const id of item.ids){const rows=item.points[id]||[];const point=nearestTime(rows,stamp);if(!point||point[1]==null||Math.abs(point[0]-stamp)>(item.to-item.from)/Math.max(1,rect.height)*12)continue;const x=item.u.valToPos(point[1],id),color=definition(id).color;
 const dot=element('div','cursorDot');dot.style.color=color;dot.style.left=x+'px';dot.style.top=y+'px';const label=element('div','tooltip',fmt(point[1]));label.style.color=color;label.style.left=Math.min(rect.width-75,Math.max(0,e.clientX-rect.left+12))+'px';label.style.top=Math.min(rect.height-25,y+12+offsetY)+'px';offsetY+=22;item.area.append(dot,label);item.cursor.push(dot,label);}
}
function nearestTime(rows,t){let lo=0,hi=rows.length;while(lo<hi){const m=(lo+hi)>>1;if(rows[m][0]<t)lo=m+1;else hi=m;}return [rows[lo],rows[lo-1]].filter(Boolean).sort((a,b)=>Math.abs(a[0]-t)-Math.abs(b[0]-t))[0];}
function flagAt(item,e){
 const rect=item.area.getBoundingClientRect(),mx=e.clientX-rect.left,my=e.clientY-rect.top,candidates=[];
 for(const id of item.ids)for(const p of item.points[id]||[]){if(p[1]==null)continue;const x=item.u.valToPos(p[1],id),y=item.u.valToPos(p[0],'x'),distance=Math.hypot(x-mx,y-my);if(distance<=10)candidates.push({id,p,distance});}
 const best=Object.values(candidates.reduce((o,c)=>{if(!o[c.id]||o[c.id].distance>c.distance)o[c.id]=c;return o;},{})).sort((a,b)=>a.distance-b.distance);if(!best.length)return;
 if(best.length>1&&best[1].distance-best[0].distance<3){const popup=element('div','flag');popup.style.left=Math.max(0,Math.min(rect.width-150,mx))+'px';popup.style.top=Math.max(0,my)+'px';for(const c of best.filter(c=>c.distance-best[0].distance<3)){const button=element('button','',definition(c.id).name);button.style.color=definition(c.id).color;button.style.display='block';button.onclick=()=>{addFlag(c);popup.remove();};popup.append(button);}item.area.append(popup);setTimeout(()=>popup.remove(),8000);return;}
 addFlag(best[0]);
}
function addFlag(c){if(!flags.some(f=>f.id===c.id&&f.raw===String(c.p[2]))){flags.push({id:c.id,t:c.p[0],value:c.p[1],raw:String(c.p[2])});flags=flags.slice(-80);localStorage.setItem('kern-flags',JSON.stringify(flags));}for(const item of plots)placeFlags(item);}
function placeFlags(item){
 for(const old of item.flags)old.remove();item.flags=[];const occupied=[];
 for(const f of flags.filter(f=>item.ids.includes(f.id)&&f.t>=item.from&&f.t<=item.to)){
 const x=item.u.valToPos(f.value,f.id),y=item.u.valToPos(f.t,'x'),def=definition(f.id);if(x<0||x>item.area.clientWidth)continue;
 const label=element('div','flag',fmt(f.value)+' '+def.unit);label.style.color=def.color;item.area.append(label);let left=Math.max(0,Math.min(item.area.clientWidth-label.offsetWidth,x+10)),top=Math.max(1,Math.min(item.area.clientHeight-23,y+8));
 for(let attempts=0;attempts<20&&occupied.some(r=>Math.abs(r.top-top)<23&&Math.abs(r.left-left)<label.offsetWidth+6);attempts++)top=Math.max(1,top-23);
 occupied.push({left,top});label.style.left=left+'px';label.style.top=top+'px';
 const svg=document.createElementNS('http://www.w3.org/2000/svg','svg');svg.setAttribute('width','1');svg.setAttribute('height','1');const path=document.createElementNS(svg.namespaceURI,'path');path.setAttribute('d',`M ${x-left-2} ${y-top-2} h 4 v 4 h -4 Z M ${x-left} ${y-top} L 0 20`);path.setAttribute('stroke',def.color);path.setAttribute('stroke-width','1');path.setAttribute('fill','none');svg.append(path);label.prepend(svg);
 label.addEventListener('dblclick',e=>{e.stopPropagation();flags=flags.filter(v=>v!==f);localStorage.setItem('kern-flags',JSON.stringify(flags));placeFlags(item);});item.flags.push(label);
 }
}
function axis(from,to){const root=$('#timeaxis');root.replaceChildren();if(!plots.length)return;const top=plots[0].header.clientHeight,height=plots[0].area.clientHeight,step=gridStep(to-from,height);for(let t=Math.ceil(from/step)*step;t<=to;t+=step){const label=element('span','',time(t));label.style.top=top+(t-from)/(to-from)*height+'px';root.append(label);}}
async function draw(){
 if(!plots.length||tab!=='charts'||document.hidden)return;
 const interval=Number($('#interval').value)*1000;const latest=snapshot.parameters.length?Math.max(...snapshot.parameters.map(p=>p.received_unix_ms||0)):Date.now();const to=latest-offset*1000,from=to-interval;
 $('#live').classList.toggle('active',offset===0);$('#history').value=86400-offset;
 const ids=[...new Set(tracks.flat())].slice(0,8),r=await api('plot',{from_ms:Math.round(from),to_ms:Math.round(to),pixels:Math.max(32,Math.min(1024,plots[0].area.clientHeight)),parameters:ids});
 if(r.truncated)note('Показана доступная часть истории — уменьшите интервал');
 for(const item of plots){
  item.from=from;item.to=to;const width=Math.max(100,item.area.clientWidth),height=Math.max(100,item.area.clientHeight);if(item.u.width!==width||item.u.height!==height)item.u.setSize({width,height});
  const times=new Set();for(const id of item.ids){const rows=r.series.find(s=>s.id===id)?.points||[];item.points[id]=rows;for(const row of rows)times.add(row[0]);}
  const ts=[...times].sort((a,b)=>a-b);const vals=item.ids.map(id=>{const points=new Map((item.points[id]||[]).map(p=>[p[0],p[1]]));return ts.map(t=>points.has(t)?points.get(t):undefined);});
  const ranges={};for(const id of item.ids){const p=definition(id),samples=(item.points[id]||[]).map(v=>v[1]).filter(v=>v!=null);if(offset===0&&current(id)?.value!=null)samples.push(current(id).value);let min=p.min,max=p.max;if(autoScale&&samples.length){const low=Math.min(...samples),high=Math.max(...samples),margin=Math.max((high-low)*.15,Math.abs(high)*.01,(p.max-p.min)*.001,.01);min=low-margin;max=high+margin;}ranges[id]={min,max};}
  item.u.batch(()=>{item.u.setData([ts,...vals],false);item.u.setScale('x',{min:from,max:to});for(const id of item.ids)item.u.setScale(id,ranges[id]);});
  item.header.querySelectorAll('.scale').forEach((scale,i)=>{const range=ranges[item.ids[i]];[...scale.children].forEach((label,n)=>label.textContent=fmt(range.min+(range.max-range.min)*n/4));});
  item.header.querySelectorAll('.title b').forEach((e,i)=>{const p=current(item.ids[i]);e.textContent=fmt(p?.value);e.style.opacity=p?.quality==='good'?'1':'.5';});placePens(item);placeFlags(item);
 }
 axis(from,to);lastDraw=performance.now();sizeDirty=false;
}
function values(){
 const root=$('#values');root.replaceChildren();for(const p of map.parameters.slice(0,8)){const v=current(p.id),a=element('article');a.append(element('small','',p.name+' · '+p.unit));const b=element('b','',fmt(v?.value));b.style.color=p.color;b.style.opacity=v?.quality==='good'?'1':'.45';b.title=v?.quality||'missing';a.append(b);root.append(a);}
}
async function refresh(){if(busy)return;busy=true;try{
 const s=await api('status');snapshot=await api('snapshot');const cal=await api('calibration');
 $('#connection').textContent='● Ядро на связи';$('#connection').className='good';$('#counts').textContent=`Входы: ${s.status.raw_batches} · Параметры: ${snapshot.parameters.length} · Расчёт: ${s.status.interpreter_connected?'подключён':'нет связи'}`;
 if(cal.revision!==revision&&!dirty){map=cal.map;revision=cal.revision;activeRevision=cal.active_revision;layout();renderMaps();}else activeRevision=cal.active_revision;
 if(activeRevision===revision&&$('#message').textContent.startsWith('Карты сохранены'))note('Калибровка '+revision+' активна');
 $('#mapState').textContent=`Версия ${revision} · активна ${activeRevision}${dirty?' · есть изменения':''}`;values();
 if(tab==='charts')await draw();
 }catch(e){$('#connection').textContent='● Нет связи';$('#connection').className='error';note(e.message);}finally{busy=false;}}
function renderMaps(){const root=$('#mapRows');root.replaceChildren();map.parameters.forEach((p,i)=>{const tr=element('tr');for(const text of [p.name,p.source,String(p.channel),p.unit,`${p.min} … ${p.max}`,p.points.map(v=>`${v.raw} → ${v.value}`).join('; ')])tr.append(element('td','',text));tr.children[0].style.color=p.color;tr.children[5].className='points';const td=element('td'),editButton=element('button','','Изменить'),remove=element('button','','Убрать');editButton.onclick=()=>edit(i);remove.onclick=()=>{map.parameters.splice(i,1);dirty=true;renderMaps();};td.append(editButton,remove);tr.append(td);root.append(tr);});}
function edit(index){editing=index;const p=index>=0?map.parameters[index]:{id:'parameter-'+(map.parameters.length+1),name:'Новый параметр',source:snapshot.sources[0]?.id||'im2480',channel:0,unit:'ед.',min:0,max:100,color:'#fabd2f',points:[{raw:0,value:0},{raw:65535,value:100}],extrapolate:false};
 const root=$('#editFields');root.replaceChildren();for(const [id,label,type,value] of [['id','ID','text',p.id],['name','Название','text',p.name],['source','Источник','text',p.source],['channel','Канал','number',p.channel],['unit','Единица','text',p.unit],['color','Цвет','color',p.color],['min','Минимум шкалы','number',p.min],['max','Максимум шкалы','number',p.max]]){const l=element('label','',label),input=element('input');input.id='edit-'+id;input.type=type;input.value=value;if(type==='number')input.step='any';if((id==='id'||id==='unit')&&index>=0)input.disabled=true;l.append(input);root.append(l);}
 const l=element('label','full','Точки: сырой код = значение, по одной на строку'),points=element('textarea');points.id='edit-points';points.value=p.points.map(v=>`${v.raw} = ${v.value}`).join('\n');l.append(points);root.append(l);
 const ext=element('label','full'),check=element('input');check.id='edit-extrapolate';check.type='checkbox';check.checked=p.extrapolate;ext.append(check,document.createTextNode('Линейно продолжать за крайними точками'));root.append(ext);$('#editError').textContent='';$('#editor').showModal();
}
$('#acceptEdit').onclick=e=>{e.preventDefault();try{const get=id=>$('#edit-'+id).value;const p={id:get('id'),name:get('name'),source:get('source'),channel:Number(get('channel')),unit:get('unit'),color:get('color'),min:Number(get('min')),max:Number(get('max')),extrapolate:$('#edit-extrapolate').checked,points:get('points').trim().split('\n').map(line=>{const v=line.split('=').map(v=>Number(v.trim().replace(',','.')));if(v.length!==2||v.some(n=>!Number.isFinite(n)))throw Error('Точки задаются как 1000 = 0');return {raw:v[0],value:v[1]};})};if(p.points.length<2||p.points.some((v,i)=>i&&v.raw<=p.points[i-1].raw)||p.min>=p.max)throw Error('Нужны две возрастающие точки raw и корректная шкала');if(editing<0&&map.parameters.some(v=>v.id===p.id))throw Error('Этот ID уже занят');if(editing<0)map.parameters.push(p);else map.parameters[editing]=p;dirty=true;renderMaps();$('#editor').close();}catch(error){$('#editError').textContent=error.message;}};
$('#newParam').onclick=()=>edit(-1);$('#reloadMap').onclick=async()=>{try{const c=await api('calibration');map=c.map;revision=c.revision;dirty=false;renderMaps();layout();}catch(e){note(e.message);}};
$('#saveMap').onclick=async()=>{try{await api('calibration',{expected_revision:revision,map});dirty=false;revision=-1;note('Карты сохранены; ожидается применение интерпретатором');await refresh();}catch(e){note(e.message);}};
async function plugins(){try{const r=await api('plugins');const root=$('#pluginRows');root.replaceChildren();for(const p of r.plugins){const a=element('article'),name=element('div');name.append(element('strong','',p.name),element('small','',p.id+' · '+p.version));a.append(name,element('span','',p.state),element('span','spacer'));if(['im2480','processor','csv','backup'].includes(p.id)){const b=element('button','',p.state==='running'?'Остановить':'Запустить');b.onclick=async()=>{try{await api('control',{id:p.id,action:p.state==='running'?'stop':'start'});setTimeout(plugins,300);}catch(e){note(e.message);}};a.append(b);}a.append(element('small','',p.detail));root.append(a);}}catch(e){note(e.message);}}
$$('nav [data-tab]').forEach(button=>button.onclick=()=>{tab=button.dataset.tab;$$('main>.screen').forEach(s=>s.classList.toggle('hidden',s.id!==tab));$$('nav [data-tab]').forEach(b=>b.classList.toggle('active',b===button));if(tab==='plugins')plugins();if(tab==='charts'){sizeDirty=true;refresh();}});
$('#autoScale').onclick=()=>{autoScale=!autoScale;$('#autoScale').classList.toggle('active',autoScale);refresh();};
$('#refreshPlugins').onclick=plugins;$('#interval').onchange=()=>{sizeDirty=true;refresh();};$('#live').onclick=()=>{offset=0;refresh();};$('#earlier').onclick=()=>{offset=Math.min(86400-Number($('#interval').value),offset+Number($('#interval').value)/2);refresh();};$('#later').onclick=()=>{offset=Math.max(0,offset-Number($('#interval').value)/2);refresh();};$('#history').oninput=e=>{offset=Math.max(0,86400-Number(e.target.value));sizeDirty=true;};$('#history').onchange=refresh;
$('#paramsMode').onclick=()=>{mode=(mode+1)%3;$('#values').classList.toggle('compact',mode===1);$('#values').classList.toggle('hidden',mode===2);sizeDirty=true;};
$('#cancelCurve').onclick=()=>$('#curvePicker').close();$('#addCurve').onclick=()=>{const id=$('#curveChoice').value;if(id&&tracks.flat().length<8){tracks[pickerTrack].push(id);layout();refresh();}$('#curvePicker').close();};
new ResizeObserver(()=>{sizeDirty=true;}).observe($('#tracks'));document.addEventListener('visibilitychange',()=>{if(!document.hidden)refresh();});
setInterval(()=>{$('#clock').textContent=time(Date.now());if(!document.hidden)refresh();},1000);
setInterval(()=>{if(sizeDirty&&!busy&&tab==='charts'&&!document.hidden)refresh();},200);
refresh();
