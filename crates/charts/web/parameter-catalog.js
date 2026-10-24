'use strict';
const usedGroupsClosed=new Set(),catalogGroupsClosed=new Set(),retainedCatalogEntries=new Map();
let catalogSelected=null,catalogBusy=false;
function parameterGroup(p){return parameterLibrary?.entries.find(e=>e.parameter.id===p.id)?.group||retainedCatalogEntries.get(p.id)?.group||'Свои параметры';}
function rememberRemovedParameter(p){
 retainedCatalogEntries.delete(p.id);retainedCatalogEntries.set(p.id,{parameter:cloneMap(p),group:parameterGroup(p),description:'Из текущей рабочей сессии'});
 while(retainedCatalogEntries.size>128)retainedCatalogEntries.delete(retainedCatalogEntries.keys().next().value);
}
function catalogEntries(){
 const entries=new Map((parameterLibrary?.entries||[]).map(e=>[e.parameter.id,e]));for(const [id,e] of retainedCatalogEntries)entries.set(id,e);return entries;
}
function catalogAddition(id){
 const entries=catalogEntries(),existing=new Map(parameterDraft.parameters.map(p=>[p.id,p])),needed=new Map(),seen=new Set(),visiting=new Set();
 function visit(key){
  if(visiting.has(key))throw Error('Цикл в зависимостях: '+key);if(seen.has(key))return;
  const p=existing.get(key)||entries.get(key)?.parameter;if(!p)throw Error('Нет определения: '+key);
  visiting.add(key);for(const dep of parameterInputs(p))visit(dep);visiting.delete(key);seen.add(key);if(!existing.has(key))needed.set(key,cloneMap(p));
 }
 visit(id);return {map:normalizeParameterMap({parameters:[...cloneMap(parameterDraft.parameters),...needed.values()]}),added:[...needed.values()],existing:[...seen].filter(k=>existing.has(k)).map(k=>existing.get(k))};
}
function renderCatalogSelection(){
 const info=$('#catalogDependency'),button=$('#catalogAdd');button.disabled=catalogBusy||applyingParameters||!catalogSelected;
 info.replaceChildren();if(!catalogSelected){info.textContent='Выберите параметр справа. Добавление изменяет только черновик.';return;}
 try{
  const plan=catalogAddition(catalogSelected),selected=catalogEntries().get(catalogSelected)?.parameter;
  info.append(element('strong','',selected.name+' · '+selected.unit),element('div','muted',providerName(selected)),element('div','','Добавится: '+plan.added.map(p=>p.name).join(', ')));
  if(plan.existing.length)info.append(element('div','muted','Уже используются: '+plan.existing.map(p=>p.name).join(', ')));
  button.disabled=catalogBusy||applyingParameters||plan.added.length===0||plan.map.parameters.length>128;
  if(plan.map.parameters.length>128)info.append(element('div','error','Превышен предел 128 параметров'));
 }catch(e){info.textContent=e.message;button.disabled=true;}
}
function renderParameterCatalog(){
 const root=$('#availableCatalog');if(!root)return;const used=new Set(parameterDraft.parameters.map(p=>p.id)),all=[...catalogEntries().values()].filter(e=>!used.has(e.parameter.id)),q=$('#catalogSearch').value.trim().toLocaleLowerCase('ru');
 const shown=all.filter(e=>(e.parameter.name+' '+e.parameter.unit+' '+e.group).toLocaleLowerCase('ru').includes(q));
 if(!shown.some(e=>e.parameter.id===catalogSelected))catalogSelected=null;
 $('#availableParameterCount').textContent=`Не используются · ${all.length}`;root.replaceChildren();
 const groups=new Map();for(const e of shown){if(!groups.has(e.group))groups.set(e.group,[]);groups.get(e.group).push(e);}
 for(const [group,entries] of groups){
  const details=element('details','catalogGroup'),summary=element('summary','',group+' · '+entries.length);details.open=!catalogGroupsClosed.has(group);details.append(summary);details.ontoggle=()=>{if(details.open)catalogGroupsClosed.delete(group);else catalogGroupsClosed.add(group);};
  for(const {parameter:p} of entries){const b=element('button','catalogParameter');b.dataset.addParameter=p.id;b.title=p.name+' · '+p.unit+' · '+providerName(p);b.classList.toggle('selected',p.id===catalogSelected);b.append(element('span','',p.name),element('small','',p.unit));b.onclick=()=>{catalogSelected=p.id;$$('.catalogParameter').forEach(row=>row.classList.toggle('selected',row.dataset.addParameter===p.id));$('#catalogError').textContent='';renderCatalogSelection();};details.append(b);}
  root.append(details);
 }
 if(!shown.length)root.append(element('p','muted',parameterLibrary?'Нет доступных параметров по этому запросу':'Каталог загружается…'));
 renderCatalogSelection();
}
async function focusParameterCatalog(){
 if(tab!=='calibration')$('nav [data-tab=calibration]').click();
 try{if(!parameterLibrary){await loadParameterLibrary();renderMaps();}$('#catalogSearch').focus();}catch(e){$('#catalogError').textContent=e.message;}
}
function setupParameterCatalog(){
 $('#newParam').onclick=focusParameterCatalog;$('#customParameter').onclick=()=>edit(-1);
 $('#libraryCatalog').textContent='Отдельные параметры →';$('#libraryCatalog').onclick=()=>{$('#libraryDialog').close();focusParameterCatalog();};
 $('#catalogSearch').oninput=()=>{catalogSelected=null;renderParameterCatalog();};$('#usedParameterSearch').oninput=renderMaps;
 $('#foldUsedGroups').onclick=()=>{const groups=[...new Set(parameterDraft.parameters.map(parameterGroup))],close=groups.some(g=>!usedGroupsClosed.has(g));for(const g of groups){if(close)usedGroupsClosed.add(g);else usedGroupsClosed.delete(g);}if(close)selectedParameter=-1;$('#foldUsedGroups').textContent=close?'Развернуть':'Свернуть';renderMaps();};
 $('#foldCatalogGroups').onclick=()=>{const groups=[...new Set([...catalogEntries().values()].map(e=>e.group))],close=groups.some(g=>!catalogGroupsClosed.has(g));for(const g of groups){if(close)catalogGroupsClosed.add(g);else catalogGroupsClosed.delete(g);}$('#foldCatalogGroups').textContent=close?'Развернуть':'Свернуть';renderParameterCatalog();};
 $('#catalogAdd').onclick=async()=>{
  if(catalogBusy||applyingParameters||!catalogSelected)return;catalogBusy=true;renderCatalogSelection();const token=JSON.stringify(parameterDraft),id=catalogSelected;
  try{const plan=catalogAddition(id);await api('parameter-presets',{action:'check_map',map:plan.map});if(applyingParameters||JSON.stringify(parameterDraft)!==token)throw Error('Черновик изменился. Проверьте состав и добавьте ещё раз.');checkpointParameters();parameterDraft=cloneMap(plan.map);selectedParameter=parameterDraft.parameters.findIndex(p=>p.id===id);usedGroupsClosed.delete(parameterGroup(parameterDraft.parameters[selectedParameter]));catalogSelected=null;changedParameters();note('Добавлено в используемые. Настройте входы и нажмите «Применить».');}
  catch(e){$('#catalogError').textContent=e.message;}finally{catalogBusy=false;renderCatalogSelection();}
 };
 loadParameterLibrary().then(()=>renderMaps()).catch(e=>{$('#catalogError').textContent=e.message;});
}
