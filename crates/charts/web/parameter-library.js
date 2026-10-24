'use strict';
let parameterLibrary=null,libraryMode='presets',librarySelected=null,libraryPreset=null,libraryPlan=null,importedPreset=null,libraryDraftToken="";
const operationNames={sum:'Сумма',difference:'Разность',ratio:'Отношение',scale:'Коэффициент и смещение',pump_flow:'Расход по ходам насоса',lookup:'Объём по таблице',weight_on_bit:'Нагрузка на долото',rate:'Скорость по временному окну'};
const reasonNames={missing_input:'Нет входных данных',invalid_input:'Вход не готов',stale_input:'Устаревшие входы',not_configured:'Не задано значение',warming_up:'Накопление истории',clock_regression:'Нарушен порядок времени',division_by_zero:'Деление на ноль',nonfinite:'Ошибка вычисления',out_of_range:'За пределами таблицы',disabled:'Отключён',processing_lag:'Ожидание интерпретатора',recovered:'История после перезапуска',stale:'Данные устарели',offline:'Источник отключён',missing:'Нет данных',bad:'Ошибка',good:'Готов'};
function providerName(p){return p.provider?.kind==='manual'?'Ввод оператора':p.provider?.kind==='calculation'?(operationNames[p.provider.operation.op]||'Расчёт'):'Канал источника';}
function parameterInputs(p){return p.provider?.kind==='calculation'?p.provider.inputs:[];}
function normalizeParameterMap(value){value.algorithm=value.parameters.some(p=>p.provider)?'graph-v1':'linear-v1';return value;}
function parameterReadiness(p){
 if(p.enabled===false&&p.provider?.operation?.op==='lookup'&&p.provider.operation.points.length<2)return 'Задайте таблицу ёмкости';
 if(p.enabled===false)return !p.provider&&(!p.points.length||p.source==='unassigned')?'Нужны канал и калибровка':'Отключён';
 if(p.provider?.kind==='manual')return p.provider.value==null?'Введите значение':'Значение задано';
 if(p.provider?.kind==='calculation'){
  const unavailable=parameterInputs(p).filter(id=>!parameterDraft.parameters.some(d=>d.id===id&&d.enabled!==false));
  if(unavailable.length)return 'Нужны: '+unavailable.map(id=>parameterDraft.parameters.find(d=>d.id===id)?.name||id).join(', ');
 }
 const live=current(p.id);return live?reasonNames[live.reason]||reasonNames[live.quality]||'Нет данных':'Ожидание входов';
}
function providerLabel(p){
 if(!p.provider)return p.source==='unassigned'?'Не назначен':inputChannel(p)?.name||`${p.source} · #${p.channel}`;
 if(p.provider.kind==='manual')return 'Ввод оператора';
 return providerName(p)+' · '+parameterInputs(p).map(id=>parameterDraft.parameters.find(d=>d.id===id)?.name||id).join(', ');
}
function field(root,id,label,value,type='number'){
 const l=element('label','',label),input=element('input');input.id=id;input.type=type;input.value=value??'';if(type==='number')input.step='any';l.append(input);root.append(l);return input;
}
function configureProviderEditor(p,root){
 const label=element('label','','Способ получения'),kind=element('select');kind.id='edit-kind';
 for(const [value,name] of [['sensor','Канал источника'],['manual','Ввод оператора'],['calculation','Расчёт']])kind.append(new Option(name,value));
 kind.value=p.provider?.kind||'sensor';label.append(kind);root.append(label);
 const enabled=field(root,'edit-enabled','Получать значения',null,'checkbox');enabled.checked=p.enabled!==false;
 const settings=element('div','providerSettings full');root.append(settings);
 const operation=p.provider?.kind==='calculation'?p.provider.operation:{op:'sum'};
 function render(){
  settings.replaceChildren();for(const id of ['source','channel'])$('#edit-'+id).closest('label').classList.toggle('hidden',kind.value!=='sensor');
  if(kind.value==='sensor'){settings.append(element('small','muted','Канал и точки настраиваются отдельно. Без проверенной калибровки параметр остаётся отключённым.'));return;}
  if(kind.value==='manual'){field(settings,'edit-manual-value','Значение (пусто — ещё не задано)',p.provider?.kind==='manual'?p.provider.value:null);settings.append(element('small','muted','Значение сохранится в версии настроек и будет доступно при восстановлении.'));return;}
  const l=element('label','','Способ расчёта'),select=element('select');select.id='edit-operation';for(const [id,name] of Object.entries(operationNames))select.append(new Option(name,id));select.value=operation.op;l.append(select);settings.append(l);
  const opSettings=element('div','providerSettings full');settings.append(opSettings);
  function renderOperation(){
   opSettings.replaceChildren();const op=select.value,previous=op===operation.op?parameterInputs(p):[];
   const count=op==='weight_on_bit'?3:['difference','ratio'].includes(op)?2:1;
   const names=op==='weight_on_bit'?['Свободный вес, т','Вес на крюке, т','Высота над забоем, м']:op==='difference'?['Уменьшаемое','Вычитаемое']:op==='ratio'?['Числитель','Знаменатель']:['Входной параметр'];
   if(op==='sum'){
    const l=element('label','full','Слагаемые (выберите несколько)'),list=element('div','sumInputs');list.id='edit-sum-inputs';
    for(const other of parameterDraft.parameters.filter(v=>v.id!==p.id)){
     const row=element('label'),check=element('input');check.type='checkbox';check.value=other.id;check.checked=previous.includes(other.id);row.append(check,document.createTextNode(other.name+' · '+other.unit));list.append(row);
    }l.append(list);opSettings.append(l);
   }else for(let n=0;n<count;n++){
    const l=element('label','',names[n]||`Вход ${n+1}`),s=element('select');s.id='edit-input-'+n;s.append(new Option('Выберите параметр…',''));
    for(const other of parameterDraft.parameters.filter(v=>v.id!==p.id))s.append(new Option(other.name+' · '+other.unit,other.id));s.value=previous[n]||'';l.append(s);opSettings.append(l);
   }
   field(opSettings,'edit-max-age','Допустимый возраст входов, с',(p.provider?.max_age_ms||5000)/1000);
   if(['scale','ratio'].includes(op))field(opSettings,'edit-factor','Коэффициент',operation.factor??(op==='ratio'?100:1));
   if(op==='scale')field(opSettings,'edit-offset','Смещение',operation.offset??0);
   if(op==='pump_flow'){field(opSettings,'edit-liters','Объём за ход, л',operation.liters_per_stroke??1);field(opSettings,'edit-efficiency','Коэффициент подачи (0…1)',operation.efficiency??1);}
   if(op==='weight_on_bit'){field(opSettings,'edit-off-bottom','Максимальная высота над забоем, м',operation.max_off_bottom??.5);opSettings.append(element('small','muted full','Нагрузка = свободный вес − вес на крюке. Выше заданной высоты — 0. Свободный вес нужно задать явно; применимость проверяет оператор.'));}
   if(op==='lookup'){
    const l=element('label','full','Таблица: вход; объём — одна точка на строку'),t=element('textarea');t.id='edit-lookup';t.rows=5;t.placeholder='0; 0\n1; 10';t.value=(operation.points||[]).map(p=>p.raw+'; '+p.value).join('\n');l.append(t);opSettings.append(l);
   }
   if(op==='rate'){
    const w=field(opSettings,'edit-window','Окно скорости, с (1…300)',operation.window_seconds??60);w.min=1;w.max=300;
    const positive=field(opSettings,'edit-positive','Учитывать только увеличение',null,'checkbox');positive.checked=operation.positive_only??true;
    opSettings.append(element('small','muted full','Скорость по приращению за полное окно. Требуется вход в метрах и единица результата м/с или м/ч. Пропуски и перезапуск источника начинают накопление заново.'));
   }
  }select.onchange=renderOperation;renderOperation();
 }
 kind.onchange=render;render();
 return result=>{
  result.enabled=enabled.checked;
  if(kind.value==='sensor'){delete result.provider;if(result.points.length<2)result.enabled=false;return result;}
  if(kind.value==='manual'){result.provider={kind:'manual',value:$('#edit-manual-value').value.trim()?numberInput($('#edit-manual-value').value):null};return result;}
  const op=$('#edit-operation').value,inputs=op==='sum'?$$('#edit-sum-inputs input:checked').map(e=>e.value):Array.from({length:op==='weight_on_bit'?3:['difference','ratio'].includes(op)?2:1},(_,n)=>$('#edit-input-'+n).value);
  if(!inputs.length||inputs.some(v=>!v))throw Error('Выберите входные параметры расчёта');
  const n=id=>numberInput($('#edit-'+id).value),operation={op};
  if(['scale','ratio'].includes(op))operation.factor=n('factor');if(op==='scale')operation.offset=n('offset');
  if(op==='pump_flow'){operation.liters_per_stroke=n('liters');operation.efficiency=n('efficiency');}
  if(op==='weight_on_bit')operation.max_off_bottom=n('off-bottom');
  if(op==='lookup')operation.points=$('#edit-lookup').value.trim()?$('#edit-lookup').value.trim().split('\n').map(line=>{const parts=line.trim().split(';');if(parts.length!==2)throw Error('Точки: вход; объём');return {raw:numberInput(parts[0]),value:numberInput(parts[1])};}):[];
  if(op==='rate'){operation.window_seconds=n('window');operation.per_hour=result.unit==='м/ч';operation.positive_only=$('#edit-positive').checked;}
  result.provider={kind:'calculation',inputs,operation,max_age_ms:n('max-age')*1000};return result;
 };
}
async function loadParameterLibrary(){parameterLibrary=await api('parameter-library');return parameterLibrary;}
function entryMap(){return new Map(parameterLibrary.entries.map(e=>[e.parameter.id,e]));}
function sectionText(sections){const names={parameters:'параметры',calculations:'расчёты',bindings:'привязки',calibrations:'калибровки',operator_values:'введённые значения'};return sections.map(s=>names[s]||s).join(', ');}
async function openParameterLibrary(mode){
 if(applyingParameters)return;try{await loadParameterLibrary();libraryMode=mode;librarySelected=null;libraryPreset=null;$('#librarySearch').value='';$('#libraryMerge').value='add';$('#libraryReplaceConfirm').checked=false;$('#libraryDialog').showModal();renderLibraryList();}catch(e){note(e.message);}
}
function renderLibraryList(){
 $('#libraryTitle').textContent=libraryMode==='catalog'?'Добавление параметра':'Варианты настроек';
 $('#libraryCatalog').classList.toggle('active',libraryMode==='catalog');$('#libraryPresets').classList.toggle('active',libraryMode==='presets');
 const root=$('#libraryList');root.replaceChildren();const q=$('#librarySearch').value.toLocaleLowerCase('ru');
 const groups=[...new Set(parameterLibrary.entries.map(e=>e.group))],group=$('#libraryGroup');const old=group.value;group.replaceChildren(new Option('Все группы',''),...groups.map(g=>new Option(g,g)));group.value=groups.includes(old)?old:'';group.classList.toggle('hidden',libraryMode!=='catalog');
 const items=libraryMode==='catalog'?parameterLibrary.entries.filter(e=>(!group.value||e.group===group.value)&&(e.parameter.name+' '+e.group).toLocaleLowerCase('ru').includes(q)).map(e=>({key:'catalog:'+e.parameter.id,name:e.parameter.name,caption:e.group+' · '+e.parameter.unit,entry:e})):[...parameterLibrary.presets.map(p=>({key:'builtin:'+p.id,name:p.name,caption:`Встроенный · v${p.version} · ${p.parameter_ids.length} параметров`,preset:p})),...parameterLibrary.custom.map(p=>({key:'custom:'+p.id,name:p.name,caption:`Мой набор · v${p.version} · ${p.count} параметров`,preset:p})),...(importedPreset?[{key:'imported',name:importedPreset.name,caption:'Из файла · ещё не сохранён',preset:importedPreset}]:[])].filter(p=>(p.name+' '+p.caption).toLocaleLowerCase('ru').includes(q));
 for(const item of items){const b=element('button','libraryItem');b.dataset.key=item.key;b.classList.toggle('active',item.key===librarySelected);b.append(element('span','',item.name),element('small','muted',item.caption));b.onclick=()=>selectLibraryItem(item);root.append(b);}
 if(!items.length){root.append(element('p','muted','Ничего не найдено'));libraryPreset=null;libraryPlan=null;$('#libraryPreview').replaceChildren();$('#libraryApply').disabled=true;$('#libraryExport').disabled=true;}
 if(!librarySelected&&items.length)selectLibraryItem(items[0]);
}
async function selectLibraryItem(item){
 librarySelected=item.key;libraryPreset=null;libraryPlan=null;$('#libraryApply').disabled=true;$('#libraryExport').disabled=true;$('#libraryDelete').disabled=true;$$('#libraryList button').forEach(b=>b.classList.toggle('active',b.dataset.key===item.key));$('#libraryError').textContent='';
 try{
  if(item.key.startsWith('custom:')){const r=await api('parameter-presets',{action:'load',id:item.preset.id});if(librarySelected!==item.key)return;libraryPreset=r.preset;}
  else if(item.entry){libraryPreset={name:item.name,description:item.entry.description,sections:['parameters','calculations'],map:{parameters:[item.entry.parameter]},roots:[item.entry.parameter.id]};}
  else if(item.key==='imported')libraryPreset=item.preset;
  else libraryPreset={...item.preset,map:{parameters:item.preset.parameter_ids.map(id=>entryMap().get(id).parameter)}};
  $('#libraryPresetName').textContent=libraryPreset.name;$('#libraryDescription').textContent=libraryPreset.description||'';
  $('#librarySections').textContent='Входит: '+sectionText(libraryPreset.sections)+'. Экраны и правила записи не включены.';
  $('#libraryDelete').disabled=!item.key.startsWith('custom:');$('#libraryStoreImport').classList.toggle('hidden',item.key!=='imported');
  renderLibraryPreview();
 }catch(e){$('#libraryError').textContent=e.message;$('#libraryApply').disabled=true;}
}
function buildLibraryPlan(){
 const byId=entryMap(),desired=new Map(),seen=new Set();const definitions=new Map(libraryPreset.map.parameters.map(p=>[p.id,p]));
 function add(id){if(seen.has(id))return;seen.add(id);const def=definitions.get(id)||byId.get(id)?.parameter;if(!def)throw Error('Нет определения: '+id);for(const input of parameterInputs(def))add(input);desired.set(id,cloneMap(def));}
 for(const p of libraryPreset.map.parameters)add(p.id);
 const mode=$('#libraryMerge').value,existing=new Map(parameterDraft.parameters.map(p=>[p.id,p]));
 const result=mode==='replace'?[]:cloneMap(parameterDraft.parameters),rows=[];let added=0,updated=0,kept=0;
 for(const [id,definition] of desired){const old=existing.get(id);let action;
  if(!old){result.push(definition);action='Добавится';added++;}
  else if(mode==='add'){action='Сохранится';kept++;}
  else{if(old.unit!==definition.unit)throw Error('Единица ID '+id+' отличается. Сохраните существующий параметр или создайте новый ID.');if(mode==='replace')result.push(definition);else result[result.findIndex(p=>p.id===id)]=definition;action='Обновится';updated++;}
  rows.push({p:mode==='add'&&old?old:definition,action,dependency:!definitions.has(id)});
 }
 const removed=mode==='replace'?parameterDraft.parameters.filter(p=>!desired.has(p.id)).length:0;
 return {map:normalizeParameterMap({parameters:result}),rows,added,updated,kept,removed};
}
function renderLibraryPreview(){
 if(!libraryPreset)return;const root=$('#libraryPreview');root.replaceChildren();
 try{libraryPlan=buildLibraryPlan();libraryDraftToken=JSON.stringify(parameterDraft);for(const {p,action,dependency} of libraryPlan.rows){const row=element('tr');for(const value of [p.name+(dependency?' · зависимость':''),p.unit,providerName(p),parameterInputs(p).map(id=>libraryPlan.map.parameters.find(d=>d.id===id)?.name||id).join(', ')||(!p.provider?'Назначить канал и калибровку':'Значение оператора'),action,parameterReadiness(p)])row.append(element('td','',value));root.append(row);}
 $('#librarySummary').textContent=`В наборе с зависимостями: ${libraryPlan.rows.length}. Добавится: ${libraryPlan.added} · обновится: ${libraryPlan.updated} · сохранится: ${libraryPlan.kept} · удалится из списка: ${libraryPlan.removed}. Итог: ${libraryPlan.map.parameters.length}.`;
 $('#libraryExport').disabled=false;$('#libraryApply').disabled=$('#libraryMerge').value==='replace'&&!$('#libraryReplaceConfirm').checked;
 $('#libraryError').textContent='';}catch(e){libraryPlan=null;$('#libraryError').textContent=e.message;$('#libraryApply').disabled=true;}
 $('#libraryReplaceLabel').classList.toggle('hidden',$('#libraryMerge').value!=='replace');
}
function setupParameterLibrary(){
 const modal=element('dialog');modal.id='libraryDialog';modal.innerHTML='<div class="libraryHeading"><h3 id="libraryTitle">Варианты настроек</h3><span class="spacer"></span><button id="libraryClose" aria-label="Закрыть варианты">×</button></div><div class="libraryTabs"><button id="libraryPresets">Наборы настроек</button><button id="libraryCatalog">Каталог параметров</button><span class="spacer"></span><button id="libraryImport">Из файла…</button><input id="libraryFile" type="file" accept=".json" hidden></div><div class="libraryWorkspace"><aside><input id="librarySearch" placeholder="Поиск…" aria-label="Поиск параметров и наборов"><select id="libraryGroup" aria-label="Группа"></select><div id="libraryList"></div><button id="libraryCustom">Свой параметр…</button></aside><section><h3 id="libraryPresetName"></h3><p id="libraryDescription" class="muted"></p><p id="librarySections" class="muted"></p><div class="libraryTable"><table><thead><tr><th>Параметр</th><th>Ед.</th><th>Способ</th><th>Что требуется</th><th>Изменение</th><th>Готовность</th></tr></thead><tbody id="libraryPreview"></tbody></table></div></section></div><div class="libraryMerge"><label>Действие <select id="libraryMerge"><option value="add">Добавить недостающие</option><option value="update">Добавить и обновить состав набора</option><option value="replace">Заменить весь список</option></select></label><label id="libraryReplaceLabel" class="hidden"><input type="checkbox" id="libraryReplaceConfirm">Заменить текущий список параметров</label></div><p id="librarySummary"></p><p id="libraryError" class="error" role="alert"></p><div class="settingsFooter"><button id="libraryExport">В файл…</button><button id="libraryDelete" disabled>Удалить мой набор</button><button id="libraryStoreImport" class="hidden">Сохранить в список</button><span class="spacer"></span><button id="libraryCancel">Отмена</button><button id="libraryApply" class="primary">В черновик</button></div>';
 document.body.append(modal);
 const save=element('dialog');save.id='savePresetDialog';save.innerHTML='<h3>Сохранить как набор</h3><div class="providerSettings"><label>Название<input id="presetName" maxlength="80"></label><label>Описание<input id="presetDescription" maxlength="500"></label></div><label class="presetOption"><input id="presetEquipment" type="checkbox">Включить текущие привязки и калибровки датчиков</label><label class="presetOption"><input id="presetValues" type="checkbox">Включить введённые значения (глубины, свободный вес и др.)</label><p class="muted">По умолчанию сохраняются определения и настройки расчётов. На другом стенде потребуются свои каналы и проверенные калибровки.</p><div id="presetSaveContents" class="presetSaveContents"></div><p id="presetSaveError" class="error"></p><div class="actions"><button id="presetSaveCancel">Отмена</button><button id="presetSaveAccept" class="primary">Сохранить в список</button></div>';
 document.body.append(save);
 $('#newParam').onclick=()=>openParameterLibrary('catalog');$('#parameterPresets').onclick=()=>openParameterLibrary('presets');
 $('#saveParameterPreset').onclick=()=>{if(!parameterDraft.parameters.length){note('Добавьте параметры');return;}$('#presetName').value='';$('#presetDescription').value='';$('#presetEquipment').checked=false;$('#presetValues').checked=false;$('#presetSaveError').textContent='';$('#presetSaveContents').replaceChildren(...parameterDraft.parameters.map(p=>element('div','',p.name+' · '+p.unit+' · '+providerName(p))));save.showModal();};
 $('#presetSaveCancel').onclick=()=>save.close();$('#presetSaveAccept').onclick=async()=>{const b=$('#presetSaveAccept');b.disabled=true;try{await api('parameter-presets',{action:'save',format:'kern-parameter-preset-v1',name:$('#presetName').value,description:$('#presetDescription').value,map:normalizeParameterMap(cloneMap(parameterDraft)),include_equipment:$('#presetEquipment').checked,include_values:$('#presetValues').checked});save.close();note('Набор сохранён. Он доступен в «Вариантах настроек».');await loadParameterLibrary();}catch(e){$('#presetSaveError').textContent=e.message;}finally{b.disabled=false;}};
 $('#libraryPresets').onclick=()=>{libraryMode='presets';librarySelected=null;renderLibraryList();};$('#libraryCatalog').onclick=()=>{libraryMode='catalog';librarySelected=null;renderLibraryList();};
 $('#librarySearch').oninput=()=>{librarySelected=null;renderLibraryList();};$('#libraryGroup').onchange=()=>{librarySelected=null;renderLibraryList();};
 $('#libraryMerge').onchange=renderLibraryPreview;$('#libraryReplaceConfirm').onchange=renderLibraryPreview;
 for(const id of ['libraryClose','libraryCancel'])$('#'+id).onclick=()=>modal.close();
 $('#libraryCustom').onclick=()=>{modal.close();edit(-1);};
 $('#libraryApply').onclick=async()=>{if(!libraryPlan)return;if(JSON.stringify(parameterDraft)!==libraryDraftToken){renderLibraryPreview();$('#libraryError').textContent='Черновик изменился. Проверьте обновлённый состав и нажмите ещё раз.';return;}const b=$('#libraryApply');b.disabled=true;try{await api('parameter-presets',{action:'check_map',map:libraryPlan.map});checkpointParameters();parameterDraft=cloneMap(libraryPlan.map);selectedParameter=Math.max(0,parameterDraft.parameters.length-libraryPlan.added);changedParameters();modal.close();note('Состав добавлен в черновик. Проверьте настройки и нажмите «Применить».');}catch(e){$('#libraryError').textContent=e.message;}finally{renderLibraryPreview();}};
 $('#libraryImport').onclick=()=>{$('#libraryFile').value='';$('#libraryFile').click();};$('#libraryFile').onchange=async e=>{try{const f=e.target.files[0];if(!f)return;if(f.size>60*1024)throw Error('Набор больше 60 КиБ');const p=JSON.parse(await f.text()),r=await api('parameter-presets',{...p,action:'validate',include_equipment:p.sections?.includes('bindings'),include_values:p.sections?.includes('operator_values')});importedPreset=r.preset;libraryMode='presets';librarySelected=null;renderLibraryList();await selectLibraryItem({key:'imported',preset:importedPreset});}catch(e){$('#libraryError').textContent=e.message;}};
 $('#libraryExport').onclick=async()=>{if(!libraryPreset)return;try{const r=await api('parameter-presets',{...libraryPreset,format:'kern-parameter-preset-v1',action:'validate',map:normalizeParameterMap({parameters:libraryPlan.rows.map(r=>cloneMap(r.p))}),include_equipment:libraryPreset.sections.includes('bindings'),include_values:libraryPreset.sections.includes('operator_values')}),url=URL.createObjectURL(new Blob([JSON.stringify(r.preset,null,2)],{type:'application/json'})),a=element('a');a.href=url;a.download='kern-preset-'+r.preset.id+'.json';a.click();setTimeout(()=>URL.revokeObjectURL(url),1000);}catch(e){$('#libraryError').textContent=e.message;}};
 $('#libraryStoreImport').onclick=async()=>{try{await api('parameter-presets',{...importedPreset,action:'save',include_equipment:importedPreset.sections.includes('bindings'),include_values:importedPreset.sections.includes('operator_values')});importedPreset=null;await loadParameterLibrary();librarySelected=null;renderLibraryList();}catch(e){$('#libraryError').textContent=e.message;}};
 $('#libraryDelete').onclick=async()=>{try{if(!librarySelected?.startsWith('custom:'))return;await api('parameter-presets',{action:'delete',id:libraryPreset.id});await loadParameterLibrary();librarySelected=null;renderLibraryList();}catch(e){$('#libraryError').textContent=e.message;}};
}
