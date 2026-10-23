'use strict';
// Presentation only: persisted calibration points always retain raw input codes.
let parameterDraft={parameters:[]}, selectedParameter=-1, parameterUndo=[], calibrationDraft=null, selectedPoint=0, applyingParameters=false;
const cloneMap=value=>JSON.parse(JSON.stringify(value));
function numberInput(text){const value=String(text).trim().replace(',','.');if(!value||!Number.isFinite(Number(value)))throw Error('Введите число');return Number(value);}
function inputChannel(p){return snapshot.sources.find(s=>s.id===p.source)?.channels.find(c=>c.id===p.channel);}
function signalKind(p){const channel=inputChannel(p);if(channel?.value_type==='unsigned'&&/^IM2480 \d+ A[123]$/.test(channel.name))return 'volts';if(channel?.value_type==='float'&&/^IM2480 \d+ F[12]$/.test(channel.name))return 'hz';return 'raw';}
function signalUnit(kind){return kind==='volts'?'В':kind==='hz'?'Гц':'код';}
function signalValue(raw,kind){return kind==='volts'?raw*12/65535:raw;}
function rawValue(signal,kind){return kind==='volts'?signal/12*65535:signal;}
function signalNow(p){const c=inputChannel(p);return c?.quality==='good'&&c.value&&typeof c.value.value==='number'?c.value.value:null;}
function checkpointParameters(){parameterUndo.push(cloneMap(parameterDraft));parameterUndo=parameterUndo.slice(-20);}
function changedParameters(){dirty=true;renderMaps();updateMapState();}
function resetParameterDraft(){parameterDraft=cloneMap(map);parameterUndo=[];selectedParameter=Math.min(selectedParameter,parameterDraft.parameters.length-1);renderMaps();}
function updateMapState(){
 $('#mapState').textContent=`Версия ${revision} · активна ${activeRevision}${dirty?' · есть изменения':''}`;
 $('#saveMap').disabled=!dirty;
}
function updateParameterSelection(){
 const selected=selectedParameter>=0&&selectedParameter<parameterDraft.parameters.length;
 for(const id of ['editParam','removeParam','calibrateParam'])$('#'+id).disabled=!selected;
 $('#undoParam').disabled=parameterUndo.length===0;
 $$('#mapRows tr').forEach((row,index)=>{row.classList.toggle('selected',index===selectedParameter);row.setAttribute('aria-selected',String(index===selectedParameter));});
}
function renderMaps(){
 const root=$('#mapRows');root.replaceChildren();
 parameterDraft.parameters.forEach((p,i)=>{
  const row=element('tr');row.tabIndex=0;row.setAttribute('role','row');row.dataset.index=i;
  for(const text of [i+1,p.name,p.unit,inputChannel(p)?.name||`${p.source} · #${p.channel}`,'—','—',`${fmt(p.min)} … ${fmt(p.max)}`])row.append(element('td','',text));
  row.children[1].style.color=p.color;row.children[4].className='signalCell';row.children[5].className='valueCell';
  row.onclick=()=>{selectedParameter=i;updateParameterSelection();};
  row.ondblclick=()=>{selectedParameter=i;updateParameterSelection();edit(i);};
  row.onkeydown=e=>{if(e.key==='Enter'){selectedParameter=i;updateParameterSelection();edit(i);}else if(e.key==='ArrowDown'||e.key==='ArrowUp'){e.preventDefault();selectedParameter=Math.max(0,Math.min(parameterDraft.parameters.length-1,i+(e.key==='ArrowDown'?1:-1)));updateParameterSelection();root.children[selectedParameter]?.focus();}};
  root.append(row);
 });updateParameterSelection();updateParameterLive();
}
function updateParameterLive(){
 if(tab==='calibration') $$('#mapRows tr').forEach((row,i)=>{const p=parameterDraft.parameters[i],raw=signalNow(p),kind=signalKind(p);row.querySelector('.signalCell').textContent=raw==null?'—':fmt(signalValue(raw,kind))+' '+signalUnit(kind);const v=current(p.id);row.querySelector('.valueCell').textContent=fmt(v?.value);row.querySelector('.valueCell').style.color=v?.quality==='good'?p.color:'var(--muted)';});
 if(calibrationDraft){const p=calibrationDraft.parameter,raw=signalNow(p);$('#calibrationLive').textContent=`Сигнал сейчас: ${raw==null?'—':fmt(signalValue(raw,calibrationDraft.kind))+' '+signalUnit(calibrationDraft.kind)} · ${fmt(current(p.id)?.value)} ${p.unit}`;$('#captureSignal').disabled=raw==null;}
}
function edit(index){
 if(applyingParameters)return;editing=index;
 const p=index>=0?parameterDraft.parameters[index]:{id:'parameter-'+Date.now().toString(36),name:'Новый параметр',source:snapshot.sources[0]?.id||'im2480',channel:0,unit:'ед.',min:0,max:100,color:'#fabd2f',points:[{raw:0,value:0},{raw:65535,value:100}],extrapolate:false};
 const root=$('#editFields');root.replaceChildren();
 for(const [id,label,type,value] of [['name','Название','text',p.name],['unit','Единица','text',p.unit],['source','Источник','text',p.source],['channel','Канал','number',p.channel],['min','Минимум шкалы','number',p.min],['max','Максимум шкалы','number',p.max],['color','Цвет','color',p.color],['id','ID параметра','text',p.id]]){
  const l=element('label','',label),input=element('input');input.id='edit-'+id;input.type=type;input.value=value;if(type==='number')input.step='any';if((id==='id'||id==='unit')&&index>=0)input.disabled=true;l.append(input);root.append(l);
 }
 $('#editTitle').textContent=index<0?'Добавить параметр':'Параметр · '+p.name;$('#editError').textContent='';$('#editor').showModal();
 $('#acceptEdit').onclick=e=>{e.preventDefault();try{
  const get=id=>$('#edit-'+id).value,channel=numberInput(get('channel'));
  const result={...p,id:get('id').trim(),name:get('name').trim(),source:get('source').trim(),channel,unit:get('unit').trim(),color:get('color'),min:numberInput(get('min')),max:numberInput(get('max'))};
  if(!/^[A-Za-z0-9_.-]{1,64}$/.test(result.id)||!/^[A-Za-z0-9_.-]{1,64}$/.test(result.source)||!result.name||!Number.isInteger(channel)||channel<0||channel>=100||result.min>=result.max)throw Error('Проверьте название, источник, канал 0–99 и шкалу');
  if(index<0&&parameterDraft.parameters.length>=32)throw Error('В прототипе не более 32 параметров');
  if(index<0&&parameterDraft.parameters.some(v=>v.id===result.id))throw Error('Этот ID уже занят');
  checkpointParameters();if(index<0){parameterDraft.parameters.push(result);selectedParameter=parameterDraft.parameters.length-1;}else parameterDraft.parameters[index]=result;
  changedParameters();$('#editor').close();
 }catch(error){$('#editError').textContent=error.message;}};
}
function validatePoints(points,kind){
 if(points.length<2||points.length>32)throw Error('Нужно от 2 до 32 точек');
 const sorted=points.map(p=>({raw:p.raw,value:p.value})).sort((a,b)=>a.raw-b.raw);
 if(sorted.some(p=>!Number.isFinite(p.raw)||!Number.isFinite(p.value)))throw Error('В каждой точке задайте сигнал и значение');
 if(sorted.some((p,i)=>i&&p.raw===sorted[i-1].raw))throw Error('У точек не должен повторяться уровень сигнала');
 if(kind==='volts'&&sorted.some(p=>p.raw<0||p.raw>65535))throw Error('Сигнал должен быть в пределах 0–12 В');
 return sorted;
}
function calibrationPoints(){
 const p=parameterDraft.parameters[selectedParameter];if(!p)return;
 calibrationDraft={parameter:cloneMap(p),points:cloneMap(p.points),kind:signalKind(p)};selectedPoint=0;
 $('#calibrationTitle').textContent='Точки калибровки · '+p.name;$('#parameterColumn').textContent='Значение, '+p.unit;
 const select=$('#signalView');select.replaceChildren();const kind=calibrationDraft.kind;
 select.append(new Option(signalUnit(kind),kind));if(kind==='volts')select.append(new Option('Сырой код','raw'));
 $('#calibrationExtrapolate').checked=p.extrapolate;$('#calibrationError').textContent='';$('#pointFactor').value='1';
 $('#calibrationDialog').showModal();renderPoints();updateParameterLive();
}
function renderPoints(){
 if(!calibrationDraft)return;const {points,kind}=calibrationDraft,root=$('#pointRows');root.replaceChildren();
 $('#signalColumn').textContent='Сигнал, '+signalUnit(kind);$('#signalScale').textContent=kind==='volts'?'Шкала входа: 0–12 В':'';
 points.forEach((p,i)=>{const tr=element('tr');tr.classList.toggle('selected',i===selectedPoint);tr.append(element('td','',i+1));
  for(const [field,value] of [['raw',signalValue(p.raw,kind)],['value',p.value]]){
   const td=element('td'),input=element('input');input.inputMode='decimal';input.value=Number.isFinite(value)?Number(value.toPrecision(12)).toString():'';input.setAttribute('aria-label',(field==='raw'?'Сигнал':'Значение')+' '+(i+1));
   input.onfocus=()=>{selectedPoint=i;$$('#pointRows tr').forEach((row,n)=>row.classList.toggle('selected',n===i));drawCalibration();};
   input.oninput=()=>{try{const value=numberInput(input.value);p[field]=field==='raw'?rawValue(value,kind):value;input.classList.remove('invalid');$('#calibrationError').textContent='';}catch{p[field]=NaN;input.classList.add('invalid');}drawCalibration();};td.append(input);tr.append(td);
  }root.append(tr);
 });$('#removePoint').disabled=points.length<=2;$('#addPoint').disabled=points.length>=32;drawCalibration();
}
function drawCalibration(){
 if(!calibrationDraft)return;
 const canvas=$('#calibrationCanvas'),rect=canvas.getBoundingClientRect();if(rect.width<1||rect.height<1)return;
 const ratio=Math.min(devicePixelRatio||1,2);canvas.width=Math.round(rect.width*ratio);canvas.height=Math.round(rect.height*ratio);
 const ctx=canvas.getContext('2d');ctx.scale(ratio,ratio);const w=rect.width,h=rect.height,b={x:65,y:24,w:w-88,h:h-78};
 ctx.fillStyle='#1d2021';ctx.fillRect(0,0,w,h);ctx.font='14px Segoe UI';
 const {parameter,kind,points}=calibrationDraft,valid=points.map((p,index)=>({x:p.value,y:signalValue(p.raw,kind),index})).filter(p=>Number.isFinite(p.x)&&Number.isFinite(p.y));
 const bounds=axis=>{const values=valid.map(p=>p[axis]);let min=Math.min(...values,0),max=Math.max(...values,1);const pad=(max-min)*.08;return [min-pad,max+pad];};
 const [x0,x1]=bounds('x'),[y0,y1]=bounds('y'),x=v=>b.x+(v-x0)/(x1-x0)*b.w,y=v=>b.y+b.h-(v-y0)/(y1-y0)*b.h;
 ctx.strokeStyle='#a8998438';ctx.lineWidth=1;ctx.setLineDash([3,4]);
 for(let n=0;n<=5;n++){const px=b.x+b.w*n/5,py=b.y+b.h*n/5;ctx.beginPath();ctx.moveTo(px,b.y);ctx.lineTo(px,b.y+b.h);ctx.moveTo(b.x,py);ctx.lineTo(b.x+b.w,py);ctx.stroke();ctx.fillStyle='#bdae93';ctx.textAlign='center';ctx.fillText(fmt(x0+(x1-x0)*n/5),px,b.y+b.h+22);ctx.textAlign='right';ctx.fillText(fmt(y1-(y1-y0)*n/5),b.x-10,py+5);}
 ctx.setLineDash([]);ctx.strokeStyle='#a89984';ctx.strokeRect(b.x,b.y,b.w,b.h);ctx.fillStyle='#ebdbb2';ctx.textAlign='center';ctx.fillText('Значение параметра, '+parameter.unit,b.x+b.w/2,h-8);
 ctx.save();ctx.translate(17,b.y+b.h/2);ctx.rotate(-Math.PI/2);ctx.fillText('Уровень сигнала, '+signalUnit(kind),0,0);ctx.restore();
 ctx.save();ctx.beginPath();ctx.rect(b.x,b.y,b.w,b.h);ctx.clip();ctx.strokeStyle=parameter.color;ctx.lineWidth=2;ctx.beginPath();valid.sort((a,b)=>a.y-b.y).forEach((p,i)=>{if(i)ctx.lineTo(x(p.x),y(p.y));else ctx.moveTo(x(p.x),y(p.y));});ctx.stroke();
 for(const p of valid){ctx.fillStyle=p.index===selectedPoint?parameter.color:'#1d2021';ctx.fillRect(x(p.x)-4,y(p.y)-4,8,8);ctx.strokeRect(x(p.x)-4,y(p.y)-4,8,8);ctx.fillStyle='#ebdbb2';ctx.textAlign='left';ctx.fillText(String(p.index+1),x(p.x)+8,y(p.y)-7);}ctx.restore();
}
function setupParameterSettings(){
 $('#newParam').onclick=()=>edit(-1);$('#editParam').onclick=()=>edit(selectedParameter);
 $('#removeParam').onclick=()=>{if(selectedParameter<0)return;checkpointParameters();parameterDraft.parameters.splice(selectedParameter,1);selectedParameter=Math.min(selectedParameter,parameterDraft.parameters.length-1);changedParameters();};
 $('#undoParam').onclick=()=>{if(!parameterUndo.length)return;parameterDraft=parameterUndo.pop();selectedParameter=Math.min(selectedParameter,parameterDraft.parameters.length-1);changedParameters();};
 $('#calibrateParam').onclick=calibrationPoints;
 $('#reloadMap').onclick=async()=>{try{const c=await api('calibration');map=c.map;revision=c.revision;activeRevision=c.active_revision;dirty=false;resetParameterDraft();layout();updateMapState();}catch(e){note(e.message);}};
 $('#saveMap').onclick=async()=>{if(applyingParameters)return;applyingParameters=true;const button=$('#saveMap');button.disabled=true;$('.parameterWorkspace').inert=true;$('#reloadMap').disabled=true;try{const submitted=cloneMap(parameterDraft);await api('calibration',{expected_revision:revision,map:submitted});dirty=false;revision=-1;note('Карты сохранены; ожидается применение интерпретатором');await refresh();}catch(e){note(e.message);}finally{applyingParameters=false;$('.parameterWorkspace').inert=false;$('#reloadMap').disabled=false;updateMapState();}};
 const close=()=>$('#calibrationDialog').close();$('#closeCalibration').onclick=close;$('#cancelCalibration').onclick=close;$('#calibrationDialog').addEventListener('close',()=>{calibrationDraft=null;});
 $('#signalView').onchange=e=>{calibrationDraft.kind=e.target.value;renderPoints();updateParameterLive();};
 $('#addPoint').onclick=()=>{const points=calibrationDraft.points;if(points.length>=32)return;const sorted=points.filter(p=>Number.isFinite(p.raw)).slice().sort((a,b)=>a.raw-b.raw);let raw=0,value=0;
  if(sorted.length>1){let a=sorted[0],b=sorted[1];for(let n=1;n<sorted.length;n++)if(sorted[n].raw-sorted[n-1].raw>b.raw-a.raw){a=sorted[n-1];b=sorted[n];}raw=(a.raw+b.raw)/2;value=(a.value+b.value)/2;}else if(sorted.length){raw=sorted[0].raw+1;value=sorted[0].value;}
  points.push({raw,value});selectedPoint=points.length-1;renderPoints();$('#pointRows tr:last-child input').focus();};
 $('#removePoint').onclick=()=>{if(calibrationDraft.points.length<=2)return;calibrationDraft.points.splice(selectedPoint,1);selectedPoint=Math.min(selectedPoint,calibrationDraft.points.length-1);renderPoints();};
 $('#captureSignal').onclick=()=>{const raw=signalNow(calibrationDraft.parameter);if(raw==null)return;calibrationDraft.points[selectedPoint].raw=raw;renderPoints();};
 $('#multiplyPoints').onclick=()=>{try{const factor=numberInput($('#pointFactor').value);if(!calibrationDraft.points.every(p=>Number.isFinite(p.value*factor)))throw Error('Некорректный коэффициент');for(const p of calibrationDraft.points)p.value*=factor;renderPoints();}catch(e){$('#calibrationError').textContent=e.message;}};
 $('#acceptCalibration').onclick=()=>{try{const points=validatePoints(calibrationDraft.points,signalKind(calibrationDraft.parameter));checkpointParameters();const p=parameterDraft.parameters[selectedParameter];p.points=points;p.extrapolate=$('#calibrationExtrapolate').checked;changedParameters();close();}catch(e){$('#calibrationError').textContent=e.message;}};
 $('#exportPoints').onclick=()=>{try{const points=validatePoints(calibrationDraft.points,signalKind(calibrationDraft.parameter)),payload={format:'kern-calibration-points-v1',unit:calibrationDraft.parameter.unit,points,extrapolate:$('#calibrationExtrapolate').checked};const url=URL.createObjectURL(new Blob([JSON.stringify(payload,null,2)],{type:'application/json'})),link=element('a');link.href=url;link.download='kern-calibration-'+calibrationDraft.parameter.id+'.json';link.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}catch(e){$('#calibrationError').textContent=e.message;}};
 $('#importPoints').onclick=()=>{$('#pointsFile').value='';$('#pointsFile').click();};
 $('#pointsFile').onchange=async e=>{try{const file=e.target.files[0];if(!file)return;if(file.size>65536)throw Error('Файл больше 64 КиБ');const v=JSON.parse(await file.text());if(v.format!=='kern-calibration-points-v1'||v.unit!==calibrationDraft.parameter.unit||!Array.isArray(v.points)||typeof v.extrapolate!=='boolean')throw Error('Неверный формат или единица параметра');calibrationDraft.points=validatePoints(v.points,signalKind(calibrationDraft.parameter));$('#calibrationExtrapolate').checked=v.extrapolate;selectedPoint=0;$('#calibrationError').textContent='';renderPoints();}catch(error){$('#calibrationError').textContent=error.message;}};
 new ResizeObserver(()=>{if(calibrationDraft)drawCalibration();}).observe($('#calibrationCanvas'));
}
