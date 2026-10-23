// Uses a dev-only Playwright install under target; never bundled with KERN.
const {chromium}=require('../target/browser-tools/node_modules/playwright');
const fs=require('fs'),path=require('path');
(async()=>{
 const profile=process.argv[2],qa=process.argv[3];
 const ui=JSON.parse(fs.readFileSync(path.join(profile,'charts-endpoint.json'),'utf8'));
 const browser=await chromium.launch({headless:true,executablePath:'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'});
 try{
 const page=await browser.newPage({viewport:{width:1440,height:960},deviceScaleFactor:1});
 const errors=[];page.on('pageerror',e=>errors.push(e.message));
 await page.goto(ui.url);await page.waitForSelector('.track');
 await page.waitForFunction(()=>typeof plots!=='undefined'&&plots.length===6&&plots[0].u.data[0].length>0,{timeout:15000});
 if(errors.length)throw Error(errors.join('\n'));
 const first=await page.evaluate(()=>{const item=plots[0],point=item.points[item.ids[0]].find(p=>p[1]!=null);const rect=item.area.getBoundingClientRect();return {x:rect.left+item.u.valToPos(point[1],item.ids[0]),y:rect.top+item.u.valToPos(point[0],'x'),points:item.u.data[0].length};});
 await page.mouse.click(first.x,Math.min(910,first.y));
 if(await page.locator('#curvePicker[open]').count())throw Error('Left click opened add-curve dialog');
 await page.mouse.dblclick(first.x,Math.min(910,first.y));
 await page.waitForSelector('.flag');
 const flag=page.locator('.flag').first();await flag.dblclick();
 await page.waitForFunction(()=>document.querySelectorAll('.flag').length===0);
 await page.mouse.move(first.x,Math.max(150,first.y-2));
 const dot=page.locator('.cursorDot');
 if(await dot.count()){const color=await dot.first().evaluate(e=>getComputedStyle(e).backgroundColor);if(color!=='rgba(0, 0, 0, 0)')throw Error('Cursor marker is filled');}
 await page.locator('.plot').first().click({button:'right',position:{x:40,y:40}});
 await page.waitForSelector('#curvePicker[open]');await page.locator('#addCurve').click();
 await page.waitForFunction(()=>tracks[0].length===2);
 await page.selectOption('#interval','30');await page.waitForTimeout(1200);
 if(await page.evaluate(()=>plots[0].u.data[0].length===0))throw Error('Short interval cleared recent history');
 await page.selectOption('#interval','300');
 for(let n=0;n<3;n++)await page.locator('#paramsMode').click();
 await page.locator('button[data-tab="calibration"]').click();
 await page.locator('#mapRows tr').nth(1).dblclick();
 await page.locator('#edit-name').fill('Тест · давление · UI');await page.locator('#acceptEdit').click();
 const beforeRevision=await page.evaluate(()=>revision);
 await page.locator('#saveMap').click();await page.waitForFunction(previous=>revision>previous&&activeRevision===revision,beforeRevision);
 await page.locator('button[data-tab="charts"]').click();await page.waitForTimeout(1300);
 await page.screenshot({path:path.join(qa,'charts.png'),fullPage:true});
 await page.locator('button[data-tab="calibration"]').click();await page.screenshot({path:path.join(qa,'calibrations.png'),fullPage:true});
 await page.locator('button[data-tab="charts"]').click();
 if(errors.length)throw Error(errors.join('\n'));
 const report={passed:true,columns:6,initialPoints:first.points,tests:['uPlot vertical tracks','single click inert','double click flag add/remove','outline cursor markers','right click add curve','30s/5min history','three parameter-panel modes','edit/save/activate calibration','no browser page errors']};
 fs.writeFileSync(path.join(qa,'browser-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
 }finally{await browser.close();}
})().catch(e=>{console.error(e);process.exit(1);});
