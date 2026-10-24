'use strict';
let imData=null,imConfig=null,imBaseConfig=null,imMap=null,imMapRevision=0,imMapDirty=false,imConfigDirty=false,imBusy=false,imTopology='',imSelection=null,imDrag=null,imIgnoreClick=0,imWireFrame=0,imDialogModules=[];
const imColors=['#83a598','#b8bb26','#d3869b','#fabd2f','#8ec07c','#fe8019'];
const imFields=['А1','А2','А3','ДОЛ','Ф1','Ф2'];
function setIMBusy(value){imBusy=value;$$('.imPort,#imDevices button').forEach(b=>b.disabled=value);$('#imPort').disabled=value;$('#imReload').disabled=value;$('#imSettings').disabled=value||!imConfig;}
function imNote(text){$('#imMessage').textContent=text;}
function imFreeSlot(modules){const used=new Set([...(imData?.state?.reserved_slots||[]),...modules.map(m=>m.slot)]);return Array.from({length:16},(_,i)=>i).find(slot=>!used.has(slot));}
function imConfigured(){return imConfig?.modules||[];}
function imSource(){return imConfig?.source_id||'im2480';}
function imBindingLabel(p){const module=imConfigured().find(m=>m.slot===Math.floor(p.channel/6));return module?`${module.address} · ${imFields[p.channel%6]}`:`Канал #${p.channel} · модуль не настроен`;}
function imSame(a,b){return JSON.stringify(a)===JSON.stringify(b);}
async function discoverIM2480(){try{const r=await api('plugins');const installed=r.plugins.some(p=>p.id==='im2480');$('#imTab').classList.toggle('hidden',!installed);return installed;}catch{return false;}}
async function refreshIM2480(force=false){
 try{imData=await api('im2480/status');$('#imTab').classList.toggle('hidden',!imData.installed);if(!imData.installed)return;
  const state=imData.state;
  if(state&&(!imConfig||force)){imConfig=cloneMap(state.config);imBaseConfig=cloneMap(state.config);imConfigDirty=false;$('#imPort').value=imConfig.port;}
  if(!imMap||force){const c=await api('calibration');imMap=cloneMap(c.map);imMapRevision=c.revision;imMapDirty=false;}
  if(state&&!imConfigDirty&&!imSame(imBaseConfig,state.config)){imConfig=cloneMap(state.config);imBaseConfig=cloneMap(state.config);$('#imPort').value=imConfig.port;}
  if(!imMapDirty&&revision>imMapRevision){imMap=cloneMap(map);imMapRevision=revision;}
  const ports=state?.ports||[];$('#imPorts').replaceChildren(...ports.map(p=>new Option(p,p)));
  const phase=state?.report?.phase;$('#imStatus').textContent=imData.running?(phase==='polling'?`● ${state.config.port} · ${state.config.baud} · приём`:`● ${phase||'Запуск…'} · ${state?.report?.last_error||''}`):'Приём остановлен';
  $('#imRun').textContent=imData.running?'Остановить приём':'Запустить приём';
  const scan=state?.scan;$('#imProgress').textContent=scan?.checked!=null?`${scan.active?'Поиск':'Поиск завершён'}: ${scan.checked} / ${scan.to-scan.from+1} · найдено ${scan.found?.length||0}${scan.cancelled?' · остановлен':''}`:'';
  $('#imStopScan').disabled=!scan?.active||imBusy;$('#imScan').disabled=!imData.running||!state||imConfigDirty||imBusy||scan?.active;
  $('#imApply').disabled=imBusy||!imData.running||(!imMapDirty&&!imConfigDirty);$('#imSettings').disabled=!imConfig||imBusy;$('#imRun').disabled=imBusy;
  $('#imDirty').textContent=imMapDirty||imConfigDirty?'Есть изменения схемы':'';
  if(imMapDirty&&revision!==imMapRevision)imNote('Карта изменена другим редактором. Перечитайте перед применением.');
  const topology=JSON.stringify([imConfig,imMap,state?.scan?.found?.map(d=>d.address)]);
  if(topology!==imTopology){imTopology=topology;renderIMBoard();}setIMBusy(imBusy);updateIMValues();
 }catch(e){imNote(e.message);}
}
function imPoint(node){const a=node.getBoundingClientRect(),b=$('#imBoard').getBoundingClientRect();return {x:a.left+a.width/2-b.left,y:a.top+a.height/2-b.top};}
function imPortNode(side,id){return [...document.querySelectorAll(side==='input'?'[data-im-input]':'[data-im-output]')].find(e=>e.dataset[side==='input'?'imInput':'imOutput']===String(id));}
function drawIMWires(){
 const board=$('#imBoard'),svg=$('#imWires');if(!imMap||tab!=='modules')return;svg.replaceChildren();svg.setAttribute('viewBox',`0 0 ${board.clientWidth} ${board.scrollHeight}`);
 const wire=(a,b,color,dashed=false)=>{const path=document.createElementNS('http://www.w3.org/2000/svg','path'),dx=(b.x-a.x)*.45;path.setAttribute('d',`M${a.x},${a.y} C${a.x+dx},${a.y} ${b.x-dx},${b.y} ${b.x},${b.y}`);path.setAttribute('stroke',color);path.setAttribute('fill','none');path.setAttribute('stroke-width','2');if(dashed)path.setAttribute('stroke-dasharray','5 4');svg.append(path);};
 for(const p of imMap.parameters){if(p.provider||p.source!==imSource()||(p.enabled===false&&p.points.length>=2))continue;const a=imPortNode('input',p.id),b=imPortNode('output',p.channel);if(a&&b)wire(imPoint(a),imPoint(b),p.channel%6===3?'#fb4934':imColors[Math.floor(p.channel/6)%imColors.length],p.enabled===false);}
 if(imDrag?.moved){const node=imPortNode(imDrag.side,imDrag.id);if(node){const rect=board.getBoundingClientRect();wire(imPoint(node),{x:imDrag.mx-rect.left,y:imDrag.my-rect.top},'#fabd2f',true);}}
}
function scheduleIMWires(){if(!imWireFrame)imWireFrame=requestAnimationFrame(()=>{imWireFrame=0;drawIMWires();});}
function imMarkChanged(){imMapDirty=true;imTopology='';renderIMBoard();$('#imApply').disabled=!imData?.running;$('#imDirty').textContent='Есть изменения схемы';}
function connectIM(a,b){if(a.side===b.side)return;const input=a.side==='input'?a:b,output=a.side==='output'?a:b,p=imMap.parameters.find(p=>p.id===input.id);if(!p)return;p.source=imSource();p.channel=Number(output.id);p.enabled=p.points.length>=2;if(!p.enabled)imNote('Канал назначен. Задайте точки калибровки в настройке параметра.');imSelection=null;imMarkChanged();}
function unlinkIM(side,id){if(!imMap)return;for(const p of imMap.parameters)if(side==='input'?p.id===id:p.source===imSource()&&p.channel===Number(id)){p.enabled=false;if(!p.points.length)p.source='unassigned';}imSelection=null;imMarkChanged();}
function makeIMPort(side,id){
 const button=element('button','imPort');button.type='button';button.disabled=imBusy;button.setAttribute('aria-label',`${side==='input'?'Вход':'Выход'} ${id}`);button.dataset[side==='input'?'imInput':'imOutput']=String(id);if(imSelection?.side===side&&imSelection.id===id)button.classList.add('chosen');
 button.onclick=()=>{if(performance.now()<imIgnoreClick||imBusy)return;const selected={side,id};if(imSelection&&imSelection.side!==side)connectIM(imSelection,selected);else{imSelection=selected;$$('.imPort').forEach(e=>e.classList.remove('chosen'));button.classList.add('chosen');}};
 button.onauxclick=e=>{if(e.button===1){e.preventDefault();if(!imBusy)unlinkIM(side,id);}};
 button.oncontextmenu=e=>{e.preventDefault();if(side==='input'&&!imBusy)unlinkIM(side,id);};
 button.onpointerdown=e=>{if(e.button===1)e.preventDefault();if(e.button!==0||imBusy)return;imDrag={side,id,pointer:e.pointerId,x:e.clientX,y:e.clientY,mx:e.clientX,my:e.clientY,moved:false};button.setPointerCapture(e.pointerId);};
 button.onpointermove=e=>{if(imDrag?.pointer!==e.pointerId)return;imDrag.mx=e.clientX;imDrag.my=e.clientY;imDrag.moved||=Math.hypot(e.clientX-imDrag.x,e.clientY-imDrag.y)>4;if(imDrag.moved)scheduleIMWires();};
 button.onpointerup=e=>{if(imDrag?.pointer!==e.pointerId)return;const drag=imDrag;imDrag=null;if(button.hasPointerCapture(e.pointerId))button.releasePointerCapture(e.pointerId);if(drag.moved){imIgnoreClick=performance.now()+200;const target=document.elementFromPoint(e.clientX,e.clientY)?.closest('[data-im-input],[data-im-output]');if(target)connectIM(drag,{side:target.dataset.imInput!=null?'input':'output',id:target.dataset.imInput??Number(target.dataset.imOutput)});scheduleIMWires();}};
 button.onpointercancel=()=>{imDrag=null;scheduleIMWires();};button.onlostpointercapture=e=>{if(imDrag?.pointer===e.pointerId){imDrag=null;scheduleIMWires();}};return button;
}
function renderIMBoard(){
 if(!imMap)return;const inputs=$('#imInputs'),devices=$('#imDevices');inputs.replaceChildren();devices.replaceChildren();
 const ordered=imMap.parameters.map((p,index)=>({p,index})).filter(({p})=>!p.provider).sort((a,b)=>Number(b.p.channel%6===3)-Number(a.p.channel%6===3));
 for(const {p,index} of ordered){const row=element('div','imInputRow'),text=element('div');text.append(element('span','',`#${String(index).padStart(2,'0')} · ${p.name}`),element('small','',p.enabled===false?'Не подключён':p.source===imSource()?imBindingLabel(p):`${p.source} · #${p.channel}`));row.append(text,makeIMPort('input',p.id));inputs.append(row);}
 if(!ordered.length)inputs.append(element('p','muted','Добавьте параметры на экране настройки.'));
 for(const m of imConfigured()){
  const card=element('article','imDevice');card.dataset.imAddress=m.address;card.style.color=imColors[m.slot%imColors.length];card.append(element('strong','',`${m.address} · IM2480 · слот ${m.slot}`),element('small','imDeviceState','Ожидание'));
  imFields.forEach((name,field)=>{const row=element('div','imOutputRow');row.append(makeIMPort('output',m.slot*6+field),element('span','',name));const value=element('b','imRaw','—');value.dataset.imChannel=m.slot*6+field;row.append(value);card.append(row);});
  card.append(element('small','imStats',''));
  const actions=element('div','imDeviceActions'),reset=element('button','','Сброс'),details=element('button','','Подробнее');reset.onclick=()=>sendIMAction({action:'reset_stats',address:m.address});details.onclick=()=>{const d=imData?.state?.report?.devices?.find(d=>d.address===m.address);$('#imDetailsText').textContent=d?`Адрес: ${d.address}\nСлот: ${d.slot}\nОпросов: ${d.polls}\nОшибок: ${d.errors}\nОшибок подряд: ${d.consecutive_errors}\nПоследний ответ: ${d.last_good_unix_ms?new Date(d.last_good_unix_ms).toLocaleString('ru-RU'):'—'}\nПоследняя ошибка: ${d.last_error||'—'}`:'Настройки ещё не применены';$('#imDetails').showModal();};actions.append(reset,details);card.append(actions);devices.append(card);
 }
 const unconfigured=(imData?.state?.scan?.found||[]).filter(found=>!imConfigured().some(m=>m.address===found.address));for(const found of unconfigured.slice(0,32)){const card=element('article','imDevice imFound');card.append(element('strong','',`${found.address} · IM2480`),element('small','',`Снимок поиска · ${new Date(found.received_unix_ms).toLocaleTimeString('ru-RU')}`));const add=element('button','','Добавить в схему');add.onclick=()=>{if(imConfig.modules.length>=16){imNote('Не более 16 модулей');return;}const slot=imFreeSlot(imConfig.modules);if(slot==null){imNote('Резерв постоянных слотов исчерпан');return;}imConfig.modules.push({address:found.address,slot});imConfigDirty=true;imTopology='';refreshIM2480();};card.append(add);devices.append(card);}
 if(unconfigured.length>32)devices.append(element('p','muted','Другие адреса: '+unconfigured.slice(32).map(d=>d.address).join(', ')+'. Добавьте нужные через настройки.'));
 scheduleIMWires();
}
function updateIMValues(){
 const source=snapshot.sources.find(s=>s.id===imSource()),state=imData?.state;
 for(const card of $$('#imDevices .imDevice[data-im-address]')){const address=Number(card.dataset.imAddress),draft=imConfigured().find(m=>m.address===address),current=state?.config?.modules.find(m=>m.address===address&&m.slot===draft?.slot),device=state?.report?.devices.find(d=>d.address===address);const applied=current&&state.config.port===imConfig.port&&state.config.baud===imConfig.baud;
  let fresh=false;for(const value of card.querySelectorAll('[data-im-channel]')){const channel=source?.channels.find(c=>c.id===Number(value.dataset.imChannel)),good=imData.running&&applied&&channel?.quality==='good'&&channel.value!=null;fresh||=good;const text=good?fmt(channel.value.value):'—';if(value.textContent!==text)value.textContent=text;}
  card.querySelector('.imDeviceState').textContent=!applied?'Не применено':fresh?'На связи':'Нет свежих данных';card.querySelector('.imStats').textContent=device?`Ошибок: ${device.errors} / опросов: ${device.polls}`:'Опрос ещё не выполнен';card.querySelector('.imDeviceActions button').disabled=!imData.running||!current||imBusy;
 }
}
async function sendIMAction(action){
 if(imBusy)return false;setIMBusy(true);$('#imApply').disabled=true;try{
  const queued=await api('im2480/command',action),until=Date.now()+30000;imNote('Команда передана владельцу COM…');
  while(Date.now()<until){await new Promise(r=>setTimeout(r,150));await refreshIM2480();const result=imData?.state?.operations?.find(v=>v.id===queued.id);if(result){imNote(result.ok?({scan:'Поиск запущен',stop_scan:'Поиск остановлен',reset_stats:'Статистика сброшена',apply:'Настройки сохранены и применены; ожидаются ответы модулей'}[action.action]||result.message):result.message);if(!result.ok)throw Error(result.message);return true;}}
  throw Error('Команда ещё выполняется. Проверьте состояние перед повтором.');
 }catch(e){imNote(e.message);return false;}finally{setIMBusy(false);await refreshIM2480();}
}
function renderIMConfigModules(){
 const root=$('#imCfgModules');root.replaceChildren();imDialogModules.forEach((m,i)=>{const row=element('tr');for(const field of ['address','slot']){const td=element('td'),input=element('input');input.type='number';input.value=m[field];input.setAttribute('aria-label',`${field} ${i+1}`);if(field==='slot')input.readOnly=true;input.oninput=()=>m[field]=input.value.trim()?Number(input.value):NaN;td.append(input);row.append(td);}const order=element('td');for(const [title,delta] of [['↑',-1],['↓',1]]){const b=element('button','',title);b.disabled=i+delta<0||i+delta>=imDialogModules.length;b.onclick=()=>{[imDialogModules[i],imDialogModules[i+delta]]=[imDialogModules[i+delta],imDialogModules[i]];renderIMConfigModules();};order.append(b);}const td=element('td'),remove=element('button','','Удалить');remove.onclick=()=>{imDialogModules.splice(i,1);renderIMConfigModules();};td.append(remove);row.append(order,td);root.append(row);});
}
function setupIM2480(){
 discoverIM2480();
 $('#imPort').onchange=e=>{if(!imConfig)return;imConfig.port=e.target.value.trim().toUpperCase();imConfigDirty=!imSame(imConfig,imBaseConfig);refreshIM2480();};
 $('#imReload').onclick=()=>{imTopology='';refreshIM2480(true);};
 $('#imScan').onclick=()=>sendIMAction({action:'scan',from:1,to:100});$('#imStopScan').onclick=()=>sendIMAction({action:'stop_scan'});
 $('#imRun').onclick=async()=>{try{await api('control',{id:'im2480',action:imData?.running?'stop':'start'});setTimeout(()=>refreshIM2480(true),350);}catch(e){imNote(e.message);}};
 $('#imParameters').onclick=()=>document.querySelector('[data-tab="calibration"]').click();
 $('#imApply').onclick=async()=>{
  if(imBusy)return;if(dirty){imNote('Сначала примените или отмените черновик на экране настройки параметров.');return;}
  if(imConfigDirty){const desired=cloneMap(imConfig);if(!await sendIMAction({action:'apply',expected:imBaseConfig,config:desired}))return;imConfig=desired;imBaseConfig=cloneMap(desired);imConfigDirty=false;}
  if(imMapDirty){setIMBusy(true);$('#imApply').disabled=true;try{await api('calibration',{expected_revision:imMapRevision,map:imMap});const current=await api('calibration');imMap=cloneMap(current.map);imMapRevision=current.revision;imMapDirty=false;note('Связи сохранены; ожидается применение интерпретатором');await refresh();}catch(e){imNote('Связи не сохранены: '+e.message);}finally{setIMBusy(false);}}
  await refreshIM2480();
 };
 $('#imSettings').onclick=()=>{if(!imConfig)return;$('#imCfgPort').value=imConfig.port;$('#imCfgBaud').replaceChildren(...[1200,2400,4800,9600,19200,38400,57600,115200].map(v=>new Option(v,v)));$('#imCfgBaud').value=imConfig.baud;$('#imCfgTimeout').value=imConfig.timeout_ms;$('#imCfgInterval').value=imConfig.poll_interval_ms;imDialogModules=cloneMap(imConfig.modules);renderIMConfigModules();$('#imCfgError').textContent='';$('#imConfigDialog').showModal();};
 $('#imCfgAdd').onclick=()=>{if(imDialogModules.length>=16)return;const slot=imFreeSlot(imDialogModules),address=Array.from({length:247},(_,i)=>i+1).find(n=>!imDialogModules.some(m=>m.address===n));if(slot==null){$('#imCfgError').textContent='Резерв постоянных слотов исчерпан';return;}imDialogModules.push({address,slot});renderIMConfigModules();};
 $('#imCfgCancel').onclick=()=>$('#imConfigDialog').close();$('#imCfgAccept').onclick=()=>{try{const next={...imConfig,port:$('#imCfgPort').value.trim().toUpperCase(),baud:Number($('#imCfgBaud').value),timeout_ms:numberInput($('#imCfgTimeout').value),poll_interval_ms:numberInput($('#imCfgInterval').value),modules:cloneMap(imDialogModules)};
  if(!/^COM[1-9]\d*$/.test(next.port)||next.timeout_ms<50||next.timeout_ms>2000||next.poll_interval_ms<100||next.poll_interval_ms>60000||!next.modules.length||new Set(next.modules.map(m=>m.address)).size!==next.modules.length||new Set(next.modules.map(m=>m.slot)).size!==next.modules.length||next.modules.some(m=>!Number.isInteger(m.address)||m.address<1||m.address>247||!Number.isInteger(m.slot)||m.slot<0||m.slot>15))throw Error('Проверьте порт, тайм-аут, адреса 1–247 и уникальные слоты 0–15');
  imConfig=next;imConfigDirty=!imSame(next,imBaseConfig);$('#imPort').value=next.port;imTopology='';$('#imConfigDialog').close();refreshIM2480();}catch(e){$('#imCfgError').textContent=e.message;}};
 $('#imDetailsClose').onclick=()=>$('#imDetails').close();new ResizeObserver(scheduleIMWires).observe($('#imBoard'));$('.imBoardScroll').addEventListener('scroll',scheduleIMWires);$('.imInputs').addEventListener('scroll',scheduleIMWires);
}
