const {chromium}=require('../target/browser-tools/node_modules/playwright');
const fs=require('fs'),path=require('path'),assert=require('assert/strict');
(async()=>{
 const [profile,qa]=process.argv.slice(2),ui=JSON.parse(fs.readFileSync(path.join(profile,'charts-endpoint.json'),'utf8'));
 const browser=await chromium.launch({headless:true,executablePath:'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'});
 try{
  const page=await browser.newPage({viewport:{width:1440,height:960}}),errors=[];page.on('pageerror',e=>errors.push(e.message));
  await page.goto(ui.url);await page.waitForSelector('#imTab:not(.hidden)');await page.locator('#imTab').click();
  await page.waitForSelector('[data-im-output="1"]');assert.equal(await page.locator('.imDevice[data-im-address]').count(),3);
  const firstPoints=await page.evaluate(()=>JSON.stringify(map.parameters.find(p=>p.id==='weight').points));
  const output=page.locator('[data-im-output="1"]'),input=page.locator('[data-im-input="weight"]');
  const a=await output.boundingBox(),b=await input.boundingBox();await page.mouse.move(a.x+a.width/2,a.y+a.height/2);await page.mouse.down();await page.mouse.move(b.x+b.width/2,b.y+b.height/2,{steps:9});await page.mouse.up();
  assert(await page.evaluate(()=>imMapDirty));assert.equal(await page.evaluate(()=>(api('calibration')).then(c=>c.map.parameters.find(p=>p.id==='weight').channel)),0);
  await page.locator('#imApply').click();await page.waitForFunction(()=>!imMapDirty&&!imBusy&&imMapRevision>=2&&activeRevision>=imMapRevision);
  assert.equal(await page.evaluate(()=>(api('calibration')).then(c=>c.map.parameters.find(p=>p.id==='weight').channel)),1);assert((await page.locator('[data-im-input=weight]').locator('..').textContent()).includes('27 · А2'));
  await input.click({button:'middle'});assert(await page.evaluate(()=>imMap.parameters.find(p=>p.id==='weight').enabled===false));
  await page.locator('#imApply').click();await page.waitForFunction(()=>snapshot.parameters.find(p=>p.id==='weight')?.quality==='disabled');
  assert.equal(await page.evaluate(()=>snapshot.parameters.find(p=>p.id==='weight').value),null);
  await output.click();await input.click();await page.locator('#imApply').click();await page.waitForFunction(()=>snapshot.parameters.find(p=>p.id==='weight')?.quality==='good');
  await page.waitForTimeout(250);await output.click();await page.locator('[data-im-input="pressure"]').click();await page.locator('#imApply').click();await page.waitForFunction(()=>!imMapDirty&&!imBusy&&activeRevision>=imMapRevision);
  assert(await page.evaluate(()=>imMap.parameters.filter(p=>p.source===imSource()&&p.channel===1&&p.enabled!==false).length>=2));
  assert.equal(await page.evaluate(()=>JSON.stringify(map.parameters.find(p=>p.id==='weight').points)),firstPoints);
  await page.locator('#imSettings').click();await page.locator('#imCfgInterval').fill('150');
  await page.locator('#imCfgModules tr').nth(1).getByRole('button',{name:'↑',exact:true}).click();
  await page.locator('#imCfgAccept').click();await page.locator('#imApply').click();await page.waitForFunction(()=>imData?.state?.config?.poll_interval_ms===150&&!imBusy);
  assert(await page.evaluate(()=>snapshot.sources.find(s=>s.id===imSource()).channels.some(c=>c.id===0&&c.name==='IM2480 27 A1')));
  await page.locator('#imScan').click();await page.waitForFunction(()=>imData?.state?.scan?.active&&imData.state.scan.checked>=1);
  await page.locator('#imStopScan').click();await page.waitForFunction(()=>imData?.state?.scan?.active===false&&!imBusy);
  assert(await page.evaluate(()=>imData.state.scan.cancelled));
  await page.locator('[data-im-address="27"]').getByRole('button',{name:'Подробнее',exact:true}).click();await page.waitForSelector('#imDetails[open]');assert((await page.locator('#imDetailsText').textContent()).includes('27'));await page.locator('#imDetailsClose').click();
  await page.locator('.imBoardScroll').evaluate(e=>e.scrollTop=0);await page.waitForTimeout(80);await page.screenshot({path:path.join(qa,'modules.png'),fullPage:true});await page.setViewportSize({width:1280,height:720});await page.screenshot({path:path.join(qa,'modules-1280.png')});
  assert.deepEqual(errors,[]);
  const report={passed:true,tests:['IM2480 tab follows installed package','configured module cards and outputs','pointer drag creates staged binding; Apply persists','middle-click disables without deleting calibration; snapshot clears value','click-two-port reconnect','one output fans out to several parameters','calibration points preserved','port timing settings hot-apply without process restart','module reorder preserves physical channel IDs','scan starts and cancels through source owner','per-module details','1280x720 layout','no browser errors']};
  fs.writeFileSync(path.join(qa,'browser-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
 }finally{await browser.close()}
})().catch(e=>{console.error(e);process.exit(1)});
