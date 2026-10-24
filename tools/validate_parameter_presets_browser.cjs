const {chromium}=require('../target/browser-tools/node_modules/playwright');
const fs=require('fs'),path=require('path'),assert=require('assert/strict');
(async()=>{
 const [profile,qa]=process.argv.slice(2),ui=JSON.parse(fs.readFileSync(path.join(profile,'charts-endpoint.json'),'utf8'));
 const browser=await chromium.launch({headless:true,executablePath:'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'});
 try{
  const page=await browser.newPage({viewport:{width:1440,height:960}}),errors=[],checks=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(ui.url);await page.waitForFunction(()=>map.parameters.length===6&&snapshot.parameters.length===6);
  await page.locator('[data-tab=calibration]').click();const initial=await page.evaluate(()=>JSON.stringify(parameterDraft));
  await page.waitForFunction(()=>parameterLibrary!==null);
  await page.locator('#appSettingsButton').click();await page.waitForSelector('#appSettings[open]');assert.equal(await page.evaluate(()=>settingsSection),'processor');await page.locator('#cancelAppSettings').click();
  async function choose(id){await page.locator('#catalogSearch').fill('');const row=page.locator('[data-add-parameter="'+id+'"]');const group=row.locator('..');if(!await group.evaluate(e=>e.open))await group.locator('summary').click();await row.click();}
  async function add(id){await choose(id);await page.locator('#catalogAdd').click();await page.waitForFunction(id=>!catalogBusy&&parameterDraft.parameters.some(p=>p.id===id),id);}
  await choose('load');assert((await page.locator('#catalogDependency').textContent()).includes('Свободный вес'));assert(await page.evaluate(()=>catalogAddition('load').added.some(p=>p.id==='hole_depth')));
  assert.equal(await page.evaluate(()=>JSON.stringify(parameterDraft)),initial);checks.push('inline available/used lists preview dependencies without changing the draft; contextual interpreter settings');
  await page.locator('#foldCatalogGroups').click();assert.equal(await page.locator('#availableCatalog details[open]').count(),0);await page.locator('#foldCatalogGroups').click();
  await page.locator('#foldUsedGroups').click();assert.equal(await page.locator('#mapRows tr[data-index]').count(),0);await page.locator('#foldUsedGroups').click();
  assert(await page.locator('#mapRows tr[data-index]').count()===6);checks.push('both thematic lists collapse and expand without removing definitions');
  await page.locator('#parameterPresets').click();await page.locator('[data-key="builtin:full-logging"]').click();await page.waitForFunction(()=>libraryPlan?.rows.length>32);
  const expected=await page.evaluate(()=>libraryPlan.map.parameters.length);assert((await page.locator('#librarySummary').textContent()).includes(String(expected)));
  await page.locator('#libraryMerge').selectOption('replace');assert(await page.locator('#libraryApply').isDisabled());await page.locator('#libraryReplaceConfirm').check();assert(await page.locator('#libraryApply').isEnabled());
  await page.setViewportSize({width:1280,height:720});const box=await page.locator('#libraryApply').boundingBox();assert(box.y+box.height<=720);await page.screenshot({path:path.join(qa,'presets-1280.png')});await page.locator('#libraryCancel').click();checks.push('full preset content/counts and explicit replacement; 1280x720 controls fit');
  await add('load');
  assert(await page.evaluate(()=>dirty&&parameterDraft.parameters.some(p=>p.id==='load')));assert.equal(await page.evaluate(()=>JSON.stringify(parameterDraft.parameters.find(p=>p.id==='weight').points)),JSON.parse(initial).parameters.find(p=>p.id==='weight').points?JSON.stringify(JSON.parse(initial).parameters.find(p=>p.id==='weight').points):'');
  assert.equal(await page.locator('[data-add-parameter="load"]').count(),0);
  await page.locator('#mapRows tr[data-parameter="free_weight"]').click();await page.locator('#removeParam').click();assert(await page.evaluate(()=>parameterDraft.parameters.some(p=>p.id==='free_weight')));
  await page.locator('#mapRows tr[data-parameter="load"]').click();await page.locator('#removeParam').click();assert.equal(await page.locator('[data-add-parameter="load"]').count(),1);await page.locator('#undoParam').click();assert.equal(await page.locator('[data-add-parameter="load"]').count(),0);
  checks.push('used definitions disappear from available; dependency removal blocked; remove and undo restore both lists');
  const beforeTransfer=await page.evaluate(()=>JSON.stringify(parameterDraft));
  const flowPoints=await page.evaluate(()=>JSON.stringify(parameterDraft.parameters.find(p=>p.id==='flow').points));
  await page.locator('#mapRows tr[data-parameter="flow"]').click();await page.locator('#removeParam').click();await add('flow');
  assert.equal(await page.evaluate(()=>JSON.stringify(parameterDraft.parameters.find(p=>p.id==='flow').points)),flowPoints);
  await page.locator('#undoParam').click();await page.locator('#undoParam').click();assert.equal(await page.evaluate(()=>JSON.stringify(parameterDraft)),beforeTransfer);
  checks.push('removing and re-adding a configured sensor preserves its calibration; undo restores original order');
  async function edit(id,fn){const index=await page.evaluate(id=>parameterDraft.parameters.findIndex(p=>p.id===id),id);await page.locator('#mapRows tr[data-index="'+index+'"]').click();await page.locator('#editParam').click();await fn();await page.locator('#acceptEdit').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);}
  async function apply(){await page.locator('#saveMap').click();await page.waitForFunction(()=>!dirty&&activeRevision===revision);}
  for(const [id,value] of [['free_weight','100'],['hole_depth','1000'],['tool_depth','1000']])await edit(id,async()=>page.locator('#edit-manual-value').fill(value));
  await apply();await page.waitForFunction(()=>snapshot.parameters.find(p=>p.id==='load')?.value===20);await page.waitForFunction(()=>[...document.querySelectorAll('#mapRows tr[data-index]')].find(r=>r.children[1].textContent==='Свободный вес колонны')?.querySelector('.readinessCell').textContent==='Значение задано');checks.push('add-missing preserves original calibration; operator inputs drive native load calculation and readiness survives omitted enabled=true');
  await add('pump_flow_1');
  await edit('pump_strokes_1',async()=>{await page.locator('#edit-source').fill('test');await page.locator('#edit-channel').fill('5');});
  const index=await page.evaluate(()=>parameterDraft.parameters.findIndex(p=>p.id==='pump_strokes_1'));await page.locator('#mapRows tr[data-index="'+index+'"]').click();await page.locator('#calibrateParam').click();
  const points=page.locator('#pointRows tr');await points.nth(0).locator('input').nth(0).fill('0');await points.nth(0).locator('input').nth(1).fill('0');await points.nth(1).locator('input').nth(0).fill('100');await points.nth(1).locator('input').nth(1).fill('100');await page.locator('#acceptCalibration').click();
  await edit('pump_flow_1',async()=>{await page.locator('#edit-liters').fill('2');await page.locator('#edit-efficiency').fill('0.9');await page.locator('#edit-enabled').check();});await apply();await page.waitForFunction(()=>Math.abs(snapshot.parameters.find(p=>p.id==='pump_flow_1')?.value-1.8)<1e-9);checks.push('new sensor starts without fabricated calibration; calibration and pump settings produce 1.8 l/s');
  await add('tank_volume_1');
  await edit('tank_volume_1',async()=>{await page.locator('#edit-lookup').fill('0; 0\n2; 20');await page.locator('#edit-enabled').check();});await apply();await page.waitForFunction(()=>Math.abs(snapshot.parameters.find(p=>p.id==='tank_volume_1')?.value-10)<1e-9);checks.push('individual tank table produces calibrated volume');
  await add('drilling_rate');
  await edit('drilling_rate',async()=>page.locator('#edit-window').fill('1'));await apply();await page.waitForFunction(()=>snapshot.parameters.find(p=>p.id==='drilling_rate')?.quality==='good');checks.push('rate editor exposes explicit history window');
  const beforeSave=await page.evaluate(()=>JSON.stringify(map));
  async function savePreset(equipment=false){await page.locator('#saveParameterPreset').click();await page.locator('#presetName').fill('Synthetic station');if(equipment)await page.locator('#presetEquipment').check();await page.locator('#presetSaveAccept').click();await page.waitForFunction(()=>!document.querySelector('#savePresetDialog').open);}
  await savePreset();await page.locator('#parameterPresets').click();let custom=await page.evaluate(()=>parameterLibrary.custom.find(p=>p.name==='Synthetic station'));assert.equal(custom.version,1);
  let data=await page.evaluate(id=>api('parameter-presets',{action:'load',id}),custom.id);assert(data.preset.map.parameters.filter(p=>!p.provider).every(p=>p.source==='unassigned'&&p.points.length===0&&!p.enabled));assert(data.preset.map.parameters.filter(p=>p.provider?.kind==='manual').every(p=>p.provider.value===null));
  await page.locator('#libraryCancel').click();assert.equal(await page.evaluate(()=>JSON.stringify(map)),beforeSave);await savePreset(true);checks.push('save-as is portable by default, leaves active map unchanged and creates immutable named versions');
  await page.locator('#parameterPresets').click();custom=await page.evaluate(()=>parameterLibrary.custom.find(p=>p.name==='Synthetic station'&&p.version===1));await page.locator(`[data-key="custom:${custom.id}"]`).click();
  await page.waitForFunction(id=>libraryPreset?.id===id,custom.id);const downloadPromise=page.waitForEvent('download');await page.locator('#libraryExport').click();const download=await downloadPromise;const file=path.join(qa,'exported-preset.json');await download.saveAs(file);assert.equal(JSON.parse(fs.readFileSync(file,'utf8')).format,'kern-parameter-preset-v1');
  await page.locator('#libraryFile').setInputFiles(file);await page.waitForFunction(()=>librarySelected==='imported'&&libraryPreset);await page.locator('#libraryStoreImport').click();await page.waitForFunction(()=>parameterLibrary.custom.length===3);checks.push('custom presets export/import, validate and explicitly store in selection list');
  await page.locator('#librarySearch').fill('no-such-parameter-xyz');assert(await page.locator('#libraryApply').isDisabled());await page.locator('#libraryCancel').click();checks.push('empty search cannot apply stale selection');
  await page.locator('#parameterPresets').click();await page.locator('[data-key="builtin:full-logging"]').click();await page.locator('#libraryMerge').selectOption('add');await page.locator('#libraryApply').click();await page.waitForFunction(()=>!document.querySelector('#libraryDialog').open);assert(await page.evaluate(()=>parameterDraft.parameters.length>32));await apply();checks.push('more than 32 parameters activate as one checked version without overwriting existing settings');
  await page.setViewportSize({width:1280,height:720});await page.waitForTimeout(100);
  const density=await page.evaluate(()=>{const rows=[...document.querySelectorAll('#mapRows tr[data-index]')],box=document.querySelector('.usedParameters').getBoundingClientRect();return {row_height:rows[0].getBoundingClientRect().height,font:parseFloat(getComputedStyle(rows[0]).fontSize),visible_rows:rows.filter(r=>{const b=r.getBoundingClientRect();return b.top>=box.top&&b.bottom<=box.bottom}).length,header_values_fit:[...document.querySelectorAll('#values article b')].every(e=>e.getBoundingClientRect().bottom<=document.querySelector('#values').getBoundingClientRect().bottom+1),available_visible:document.querySelector('.availableParameters').getBoundingClientRect().right<=innerWidth};});
  assert(density.row_height<=26&&density.font>=14&&density.visible_rows>=14&&density.available_visible&&density.header_values_fit);
  await page.locator('.usedParameters .tableScroll').evaluate(e=>e.scrollTop=0);await page.screenshot({path:path.join(qa,'parameter-workspace-1280.png')});checks.push('compact two-pane workspace: 14px text, rows <=26px and both lists visible at 1280x720');
  assert(await page.locator('#parameterTable').evaluate(e=>e.scrollWidth<=e.closest('.tableScroll').clientWidth+2));await page.screenshot({path:path.join(qa,'parameter-settings.png'),fullPage:true});assert.deepEqual(errors,[]);checks.push('no browser errors');
  fs.writeFileSync(path.join(qa,'browser-report.json'),JSON.stringify({passed:true,checks,density},null,2)+'\n');console.log(JSON.stringify({passed:true,checks:checks.length}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1)});
