'use strict';
const defaultChartAppearance=Object.freeze({tile_left:0,tile_right:35,tile_overall:100});
let chartAppearance={...defaultChartAppearance},appearanceSaved={revision:0,charts:{...defaultChartAppearance}},appearanceDraft=null,settingsSection='general',settingsPlugins=[],settingsPluginNotice='',settingsSaving=false,appearanceTimer=0,previewTracks=[];
const settingsNames={general:'Приложение',charts:'Графики',processor:'Интерпретатор',im2480:'Модули IM2480',csv:'Запись CSV',backup:'Восстановительный резерв','kern.shell':'Системная оболочка'};
function pluginSettingsButton(id){
 const b=element('button','iconButton','⚙');b.type='button';b.title='Настройки · '+(settingsNames[id]||id);b.setAttribute('aria-label',b.title);b.onclick=()=>openAppSettings(id);return b;
}
function redrawAppearance(){
 if(appearanceTimer)return;
 appearanceTimer=setTimeout(()=>{appearanceTimer=0;if(tab==='charts'&&!document.hidden)for(const p of plots)p.u.redraw(false,false);drawAppearancePreview();},32);
}
function acceptAppearance(value){
 if(value.revision<appearanceSaved.revision)return;
 appearanceSaved={revision:value.revision,charts:{...value.charts}};
 if(!appearanceDraft){chartAppearance={...value.charts};redrawAppearance();}
}
async function loadAppearance(){acceptAppearance(await api('presentation-settings'));}
function appearanceDirty(){return appearanceDraft&&Object.keys(defaultChartAppearance).some(k=>appearanceDraft[k]!==appearanceSaved.charts[k]);}
function updateAppearanceActions(){
 $('#saveAppSettings').disabled=settingsSaving||!appearanceDirty();$('#resetAppSettings').disabled=settingsSaving;$('#cancelAppSettings').disabled=settingsSaving;$('#closeAppSettings').disabled=settingsSaving;$('#reloadAppSettings').disabled=settingsSaving;
 $('#settingsSections').inert=settingsSaving;$('#settingsContent').inert=settingsSaving;
}
async function openAppSettings(section='general'){
 try{
  if($('#appSettings').open){settingsSection=section;renderAppSettings();return;}
  const [preferences,components]=await Promise.all([api('presentation-settings'),api('plugins').catch(e=>({plugins:[],unavailable:e.message}))]);acceptAppearance(preferences);settingsPlugins=components.plugins;settingsPluginNotice=components.unavailable||'';
  appearanceDraft={...appearanceSaved.charts};settingsSection=section;$('#appearanceError').textContent='';$('#appSettings').showModal();renderAppSettings();
 }catch(e){note(e.message);}
}
function leaveSettingsFor(tabName,action){
 $('#appSettings').close();$(`nav [data-tab="${tabName}"]`).click();if(action)action();
}
function settingsLink(text,tabName,action){const b=element('button','',text);b.onclick=()=>leaveSettingsFor(tabName,action);return b;}
function renderAppSettings(){
 const sections=$('#settingsSections');sections.replaceChildren();
 const ids=['general','charts',...settingsPlugins.map(p=>p.id).filter(id=>id!=='charts')];
 if(!ids.includes(settingsSection))settingsSection='general';
 for(const id of ids){const b=element('button','settingsSection',settingsNames[id]||settingsPlugins.find(p=>p.id===id)?.name||id);b.classList.toggle('active',id===settingsSection);b.setAttribute('aria-current',id===settingsSection?'page':'false');b.onclick=()=>{settingsSection=id;renderAppSettings();};b.dataset.settingsSection=id;sections.append(b);}
 const content=$('#settingsContent');content.replaceChildren(element('h3','',settingsNames[settingsSection]||settingsPlugins.find(p=>p.id===settingsSection)?.name||settingsSection));previewTracks=[];
 if(settingsSection==='charts'){
  content.append(element('p','muted','Плитка и границы дорожек. Изменения видны сразу; сохранение — отдельной кнопкой.'));
  const preview=element('canvas');preview.id='appearancePreview';preview.setAttribute('aria-label','Предпросмотр двух соседних графиков');content.append(preview);
  for(const [id,label,hint] of [['tile_left','Градиент слева','Яркость левого края'],['tile_right','Градиент справа','Яркость правого края'],['tile_overall','Общий градиент','Интенсивность перехода и оттенков плитки']]){
   const row=element('label','appearanceSlider'),caption=element('span'),output=element('output','',appearanceDraft[id]+'%'),input=element('input');input.type='range';input.min='0';input.max='100';input.step='1';input.value=appearanceDraft[id];input.id='appearance-'+id;input.setAttribute('aria-label',label);output.htmlFor=input.id;caption.append(element('span','',label),output);input.oninput=()=>{appearanceDraft[id]=Number(input.value);chartAppearance={...appearanceDraft};output.textContent=input.value+'%';$('#appearanceError').textContent='';updateAppearanceActions();redrawAppearance();};row.append(caption,input,element('small','muted',hint));content.append(row);
  }
 }else if(settingsSection==='general'){
  content.append(element('p','','KERN · Регистрация параметров'),element('p','muted','Выберите плагин слева. Шестерёнка рядом с плагином открывает этот же раздел напрямую.'),settingsLink('Управление плагинами','plugins'));if(settingsPluginNotice)content.append(element('p','muted','Реестр плагинов недоступен: '+settingsPluginNotice));
 }else{
  const p=settingsPlugins.find(p=>p.id===settingsSection);content.append(element('p','muted',`${p?.id||settingsSection} · ${p?.version||''} · ${p?.state||''}`));
  if(settingsSection==='processor')content.append(element('p','','Параметры, наборы, источники, расчёты и карты калибровок.'),settingsLink('Настройка параметров…','calibration'));
  else if(settingsSection==='im2480')content.append(element('p','','Подключение модулей, COM-порт, скорость опроса и диагностика.'),settingsLink('Модули и настройки линии…','modules',async()=>{await refreshIM2480();if(!$('#imSettings').disabled)$('#imSettings').click();}));
  else if(settingsSection==='csv'||settingsSection==='backup')content.append(element('p','',settingsSection==='csv'?'Запись параметров в CSV.':'Сохранение событий для восстановления.'),element('p','muted','Периоды записи пока не настраиваются. Запуск и остановка доступны на экране плагинов.'),settingsLink('Управление плагином','plugins'));
  else content.append(element('p','muted',settingsSection==='kern.shell'?'Обязательная системная оболочка. Настраиваемых опций этого компонента пока нет.':'Этот плагин пока не предоставил отдельную форму настроек.'),settingsLink('Управление плагинами','plugins'));
 }
 $('#resetAppSettings').classList.toggle('hidden',settingsSection!=='charts');updateAppearanceActions();redrawAppearance();
}
function drawAppearancePreview(){
 const canvas=$('#appearancePreview');if(!canvas||!$('#appSettings').open)return;
 const ratio=uPlot.pxRatio,width=Math.round(canvas.clientWidth*ratio),height=Math.round(canvas.clientHeight*ratio);if(width<1||height<1)return;
 if(canvas.width!==width||canvas.height!==height){canvas.width=width;canvas.height=height;}
 const ctx=canvas.getContext('2d');previewTracks=previewTracks.length?previewTracks:[{from:1700000000000,to:1700000300000,tileSeed:17},{from:1700000000000,to:1700000300000,tileSeed:18}];
 previewTracks.forEach((item,i)=>{const left=i*width/2,w=width/2;tileGrid(item,{ctx,bbox:{left,top:0,width:w,height},valToPos:t=>(t-item.from)/(item.to-item.from)*height});ctx.strokeStyle=i?'#83a598':'#fabd2f';ctx.lineWidth=1.5*ratio;ctx.beginPath();for(let y=0;y<=height;y+=4*ratio){const x=left+w*(.45+.12*Math.sin(y/ratio/24+i));if(y===0)ctx.moveTo(x,y);else ctx.lineTo(x,y);}ctx.stroke();});
}
function setupAppSettings(){
 const dialog=element('dialog');dialog.id='appSettings';dialog.innerHTML='<div class="appSettingsHeading"><h2>Настройки</h2><span class="spacer"></span><button id="reloadAppSettings">Перечитать</button><button id="closeAppSettings" class="iconButton" aria-label="Закрыть настройки">×</button></div><div class="appSettingsBody"><aside id="settingsSections" aria-label="Разделы настроек"></aside><section id="settingsContent"></section></div><p id="appearanceError" role="status"></p><div class="settingsFooter"><button id="resetAppSettings">По умолчанию</button><span class="spacer"></span><button id="cancelAppSettings">Отмена</button><button id="saveAppSettings" class="primary" disabled>Сохранить</button></div>';document.body.append(dialog);
 $('#appSettingsButton').onclick=()=>openAppSettings();$('#chartSettingsButton').onclick=()=>openAppSettings('charts');
 for(const id of ['closeAppSettings','cancelAppSettings'])$('#'+id).onclick=()=>dialog.close();
 dialog.addEventListener('cancel',e=>{if(settingsSaving)e.preventDefault();});dialog.addEventListener('close',()=>{appearanceDraft=null;chartAppearance={...appearanceSaved.charts};previewTracks=[];redrawAppearance();});
 $('#resetAppSettings').onclick=()=>{appearanceDraft={...defaultChartAppearance};chartAppearance={...appearanceDraft};$('#appearanceError').textContent='';renderAppSettings();};
 $('#reloadAppSettings').onclick=async()=>{try{const value=await api('presentation-settings');acceptAppearance(value);appearanceDraft={...appearanceSaved.charts};chartAppearance={...appearanceDraft};$('#appearanceError').textContent='Настройки перечитаны';renderAppSettings();}catch(e){$('#appearanceError').textContent=e.message;}};
 $('#saveAppSettings').onclick=async()=>{if(settingsSaving||!appearanceDirty())return;settingsSaving=true;updateAppearanceActions();try{const value=await api('presentation-settings',{expected_revision:appearanceSaved.revision,charts:appearanceDraft});acceptAppearance(value);appearanceDraft={...appearanceSaved.charts};chartAppearance={...appearanceDraft};$('#appearanceError').textContent='Сохранено';redrawAppearance();}catch(e){$('#appearanceError').textContent=e.message;}finally{settingsSaving=false;updateAppearanceActions();}};
 new ResizeObserver(()=>{if(dialog.open)redrawAppearance();}).observe($('#settingsContent'));
 loadAppearance().catch(e=>note(e.message));
}
