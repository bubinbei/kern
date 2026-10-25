const {chromium}=require('../target/browser-tools/node_modules/playwright');
const fs=require('fs'),path=require('path'),assert=require('assert/strict');
(async()=>{
 const [profile,qa]=process.argv.slice(2),ui=JSON.parse(fs.readFileSync(path.join(profile,'charts-endpoint.json'))),browser=await chromium.launch({headless:true,executablePath:'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'});
 try{
  const page=await browser.newPage({viewport:{width:1280,height:720}}),errors=[];page.on('pageerror',e=>errors.push(e.message));await page.goto(ui.url);
  await page.waitForFunction(()=>typeof openAppSettings==='function');
  await page.evaluate(async()=>{await openAppSettings('general');settingsPlugins.push({id:'backup',name:'Восстановительный резерв',state:'running',version:'0.1.0'});settingsSection='backup';renderAppSettings();});
  await page.waitForSelector('#backupDirectory');assert.equal(await page.locator('#backupCompression option').count(),4);assert(await page.locator('#saveAppSettings').isDisabled());
  const initial=await page.evaluate(()=>api('backup-settings'));
  await page.locator('#backupCompression').selectOption('balanced');assert(await page.locator('#saveAppSettings').isEnabled());await page.locator('#cancelAppSettings').click();assert.equal((await page.evaluate(()=>api('backup-settings'))).settings.revision,initial.settings.revision);
  await page.evaluate(async()=>{await openAppSettings('general');settingsPlugins.push({id:'backup'});settingsSection='backup';renderAppSettings();});
  await page.locator('#backupCompression').selectOption('balanced');await page.locator('#saveAppSettings').click();await page.waitForFunction(()=>document.querySelector('#appearanceError').textContent==='Сохранено');
  assert.equal((await page.evaluate(()=>api('backup-settings'))).settings.compression,'balanced');
  // Exercise picker integration without automating another process through a browser.
  await page.route('**/api/backup-folder',route=>route.fulfill({json:{result:'backup_folder',folder:null}}));const prior=await page.locator('#backupDirectory').inputValue();await page.locator('#backupChoose').click();await page.waitForFunction(()=>!settingsSaving);assert.equal(await page.locator('#backupDirectory').inputValue(),prior);
  await page.unroute('**/api/backup-folder');await page.route('**/api/backup-folder',route=>route.fulfill({json:{result:'backup_folder',folder:qa}}));await page.locator('#backupChoose').click();await page.waitForFunction(()=>!settingsSaving);assert.equal(await page.locator('#backupDirectory').inputValue(),qa);await page.locator('#reloadAppSettings').click();await page.waitForFunction(()=>document.querySelector('#appearanceError').textContent==='Настройки перечитаны');
  await page.evaluate(async()=>{const s=await api('backup-settings');await api('backup-settings',{expected_revision:s.settings.revision,directory:s.settings.directory,compression:'compact'});});await page.locator('#backupCompression').selectOption('none');await page.locator('#saveAppSettings').click();await page.waitForFunction(()=>document.querySelector('#appearanceError').textContent.includes('reload'));
  assert.equal(await page.locator('#backupCompression').inputValue(),'none');await page.locator('#reloadAppSettings').click();await page.waitForFunction(()=>document.querySelector('#backupCompression').value==='compact');
  assert(await page.locator('#saveAppSettings').evaluate(e=>e.getBoundingClientRect().bottom<=innerHeight));await page.screenshot({path:path.join(qa,'backup-settings.png')});assert.deepEqual(errors,[]);
  console.log(JSON.stringify({checks:['Browser: four profiles, Save/Cancel/reload/stale conflict, picker result/cancel, active path and 1280x720 layout'],page_errors:errors}));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1)});
