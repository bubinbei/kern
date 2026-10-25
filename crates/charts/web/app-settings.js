'use strict';
const defaultChartAppearance=Object.freeze({tile_left:0,tile_right:35,tile_overall:100});
let chartAppearance={...defaultChartAppearance},appearanceSaved={revision:0,charts:{...defaultChartAppearance}},appearanceDraft=null,settingsSection='general',settingsPlugins=[],settingsPluginNotice='',settingsSaving=false,appearanceTimer=0,previewTracks=[];
const settingsNames={general:'Приложение',charts:'Графики',processor:'Интерпретатор',im2480:'Модули IM2480',csv:'Запись CSV',backup:'Восстановительный резерв','kern.shell':'Системная оболочка'};
let backupSaved=null,backupDraft=null,backupLoadError='',backupStatusTimer=0;
const backupProfiles={none:['Без сжатия','Без сжатия блоков. Минимум вычислений, больше места на диске.'],fast:['Быстрое · слабый ПК','Zstd −3, один поток. Начальный выбор для Celeron J4125; нагрузку нужно проверить на самом ПК.'],balanced:['Сбалансированное','Zstd 1, один поток. Меньше размер при умеренной работе процессора.'],compact:['Компактное','Zstd 3, один поток. Больше вычислений ради уменьшения файла.']};
async function loadBackupSettings(){try{backupSaved=await api('backup-settings');backupDraft={...backupSaved.settings};backupLoadError='';}catch(e){backupSaved=null;backupDraft=null;backupLoadError=e.message;}}
function backupDirty(){return backupDraft&&backupSaved&&(backupDraft.directory!==backupSaved.settings.directory||backupDraft.compression!==backupSaved.settings.compression||backupDraft.layout!==backupSaved.settings.layout);}
function updateBackupStatus(value){
 const output=$('#backupActual');if(!output)return;const s=value.status,fresh=s&&Date.now()-s.updated_unix_ms<5000;
 output.textContent=s?.file||value.file;
 $('#backupDesired').textContent=value.file;
 $('#backupProgress').textContent=s?`${fresh?'Запись':'Последний статус'}: ${s.position} / ${s.through} событий · архив ${((s.archive_bytes??s.bytes)/1048576).toFixed(2)} МиБ${s.active_revision!==value.settings.revision?' · новые настройки ещё не применены':''}${s.error?' · '+s.error:''}`:'Плагин ещё не сообщил о записи. Запустите его на экране плагинов.';
}
function renderBackupSettings(content){
 clearInterval(backupStatusTimer);
 if(!backupDraft){content.append(element('p','',backupLoadError||'Настройки резерва недоступны'));return;}
 content.append(element('p','muted','Сырые данные и изменения настроек сохраняются полностью. Новая папка получит всю доступную историю ядра.'));
 const formatLabel=element('label','backupLabel','Состав резерва'),format=element('select');format.id='backupLayout';format.setAttribute('aria-label','Состав резерва');for(const [id,text]of [['hourly_v2','Сырые данные + параметры раз в 10 с · часовые файлы'],['legacy','Полный журнал v1 · совместимость']]){const o=element('option','',text);o.value=id;format.append(o);}format.value=backupDraft.layout;format.onchange=()=>{backupDraft.layout=format.value;updateAppearanceActions();};formatLabel.append(format);content.append(formatLabel,element('p','muted','Часовые файлы: новая папка каждые сутки, новый файл также при активации калибровки. Сырые данные сохраняются полностью; промежуточные результаты проверяются по контрольным суммам при восстановлении. Смена состава перезапустит управляемый плагин.'));
 const label=element('label','backupLabel','Папка сохранения'),row=element('div','backupFolderRow'),input=element('input'),choose=element('button','','Выбрать…'),reset=element('button','','Папка профиля');input.id='backupDirectory';input.type='text';input.value=backupDraft.directory;input.placeholder='Папка backup внутри профиля';input.setAttribute('aria-label','Папка сохранения резерва');input.oninput=()=>{backupDraft.directory=input.value;$('#appearanceError').textContent='';updateAppearanceActions();};
 choose.id='backupChoose';choose.onclick=async()=>{settingsSaving=true;updateAppearanceActions();try{const v=await api('backup-folder',{});if(v.folder!==null){backupDraft.directory=v.folder;input.value=v.folder;}}catch(e){$('#appearanceError').textContent=e.message;}finally{settingsSaving=false;updateAppearanceActions();}};
 reset.id='backupDefault';reset.onclick=()=>{backupDraft.directory='';input.value='';updateAppearanceActions();};row.append(input,choose,reset);label.append(row);content.append(label,element('p','muted','Во внешней папке создаётся отдельная подпапка этого профиля KERN. Старые файлы остаются на месте.'));
 const compression=element('label','backupLabel','Сжатие'),select=element('select'),hint=element('p','muted',backupProfiles[backupDraft.compression][1]);select.id='backupCompression';select.setAttribute('aria-label','Сжатие резерва');for(const [key,[text]]of Object.entries(backupProfiles)){const o=element('option','',text);o.value=key;select.append(o);}select.value=backupDraft.compression;select.onchange=()=>{backupDraft.compression=select.value;hint.textContent=backupProfiles[select.value][1];updateAppearanceActions();};compression.append(select);content.append(compression,hint);
 content.append(element('p','muted','Фактический файл записи'),Object.assign(element('div','backupPath'),{id:'backupActual'}),element('p','muted','Путь по сохранённым настройкам'),Object.assign(element('div','backupPath'),{id:'backupDesired'}),Object.assign(element('p','muted'),{id:'backupProgress'}),settingsLink('Управление плагином','plugins'));updateBackupStatus(backupSaved);
 backupStatusTimer=setInterval(async()=>{if(!$('#appSettings').open||settingsSection!=='backup'){clearInterval(backupStatusTimer);return;}try{updateBackupStatus(await api('backup-settings'));}catch(e){$('#backupProgress').textContent=e.message;}},1500);
}
function settingsContext(){return {charts:'charts',calibration:'processor',modules:'im2480',plugins:'general'}[tab]||'general';}
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
 $('#saveAppSettings').disabled=settingsSaving||!(appearanceDirty()||backupDirty());$('#resetAppSettings').disabled=settingsSaving;$('#cancelAppSettings').disabled=settingsSaving;$('#closeAppSettings').disabled=settingsSaving;$('#reloadAppSettings').disabled=settingsSaving;
 $('#settingsSections').inert=settingsSaving;$('#settingsContent').inert=settingsSaving;
}
async function openAppSettings(section='general'){
 try{
  if($('#appSettings').open){settingsSection=section;renderAppSettings();return;}
  const [preferences,components]=await Promise.all([api('presentation-settings'),api('plugins').catch(e=>({plugins:[],unavailable:e.message}))]);acceptAppearance(preferences);settingsPlugins=components.plugins;settingsPluginNotice=components.unavailable||'';
  await loadBackupSettings();appearanceDraft={...appearanceSaved.charts};settingsSection=section;$('#appearanceError').textContent='';$('#appSettings').showModal();renderAppSettings();
 }catch(e){note(e.message);}
}
function leaveSettingsFor(tabName,action){
 $('#appSettings').close();$(`nav [data-tab="${tabName}"]`).click();if(action)action();
}
function settingsLink(text,tabName,action){const b=element('button','',text);b.onclick=()=>leaveSettingsFor(tabName,action);return b;}
function renderAppSettings(){
 const sections=$('#settingsSections');sections.replaceChildren();
 const ids=[...new Set(['general','charts','processor',...settingsPlugins.map(p=>p.id)])];
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
  content.append(element('p','','KERN · Регистрация параметров'),element('p','muted','Выберите плагин слева. Кнопка ⋮ открывает настройки текущего экрана.'),settingsLink('Управление плагинами','plugins'));if(settingsPluginNotice)content.append(element('p','muted','Реестр плагинов недоступен: '+settingsPluginNotice));
 }else{
  const p=settingsPlugins.find(p=>p.id===settingsSection);content.append(element('p','muted',`${p?.id||settingsSection} · ${p?.version||''} · ${p?.state||''}`));
  if(settingsSection==='processor')content.append(element('p','','Параметры, наборы, источники, расчёты и карты калибровок.'),settingsLink('Настройка параметров…','calibration'));
  else if(settingsSection==='im2480')content.append(element('p','','Подключение модулей, COM-порт, скорость опроса и диагностика.'),settingsLink('Модули и настройки линии…','modules',async()=>{await refreshIM2480();if(!$('#imSettings').disabled)$('#imSettings').click();}));
  else if(settingsSection==='backup')renderBackupSettings(content);
  else if(settingsSection==='csv')content.append(element('p','',settingsSection==='csv'?'Запись параметров в CSV.':'Сохранение событий для восстановления.'),element('p','muted','Периоды записи пока не настраиваются. Запуск и остановка доступны на экране плагинов.'),settingsLink('Управление плагином','plugins'));
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
 $('#appSettingsButton').onclick=()=>openAppSettings(settingsContext());
 for(const id of ['closeAppSettings','cancelAppSettings'])$('#'+id).onclick=()=>dialog.close();
 dialog.addEventListener('cancel',e=>{if(settingsSaving)e.preventDefault();});dialog.addEventListener('close',()=>{appearanceDraft=null;backupDraft=null;clearInterval(backupStatusTimer);chartAppearance={...appearanceSaved.charts};previewTracks=[];redrawAppearance();});
 $('#resetAppSettings').onclick=()=>{appearanceDraft={...defaultChartAppearance};chartAppearance={...appearanceDraft};$('#appearanceError').textContent='';renderAppSettings();};
 $('#reloadAppSettings').onclick=async()=>{try{const value=await api('presentation-settings');acceptAppearance(value);await loadBackupSettings();appearanceDraft={...appearanceSaved.charts};chartAppearance={...appearanceDraft};$('#appearanceError').textContent='Настройки перечитаны';renderAppSettings();}catch(e){$('#appearanceError').textContent=e.message;}};
 $('#saveAppSettings').onclick=async()=>{if(settingsSaving||!(appearanceDirty()||backupDirty()))return;settingsSaving=true;updateAppearanceActions();try{if(appearanceDirty()){const value=await api('presentation-settings',{expected_revision:appearanceSaved.revision,charts:appearanceDraft});acceptAppearance(value);appearanceDraft={...appearanceSaved.charts};chartAppearance={...appearanceDraft};}if(backupDirty()){const value=await api('backup-settings',{expected_revision:backupSaved.settings.revision,directory:backupDraft.directory,compression:backupDraft.compression,layout:backupDraft.layout});backupSaved=value;backupDraft={...value.settings};updateBackupStatus(value);} $('#appearanceError').textContent='Сохранено';redrawAppearance();}catch(e){$('#appearanceError').textContent=e.message;}finally{settingsSaving=false;updateAppearanceActions();}};
 new ResizeObserver(()=>{if(dialog.open)redrawAppearance();}).observe($('#settingsContent'));
 loadAppearance().catch(e=>note(e.message));
}
