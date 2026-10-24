// Dev-only real browser; the fixture is synthetic and owned by the Python runner.
const {chromium}=require('../target/browser-tools/node_modules/playwright');
const fs=require('fs'),path=require('path'),assert=require('assert/strict');
(async()=>{
 const [profile,qa]=process.argv.slice(2),ui=JSON.parse(fs.readFileSync(path.join(profile,'charts-endpoint.json'),'utf8'));
 const browser=await chromium.launch({headless:true,executablePath:'C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe'});
 try{
  const page=await browser.newPage({viewport:{width:1440,height:960}}),errors=[],plotRequests=[];
  page.on('pageerror',e=>errors.push(e.message));page.on('request',r=>{if(r.url().endsWith('/api/plot'))plotRequests.push(Date.now())});
  await page.goto(ui.url);await page.waitForFunction(()=>typeof plots!=='undefined'&&plots.length===6&&plots[0].u.data[0].length>0);
  const tiles=await page.evaluate(()=>{
   const ratio=uPlot.pxRatio,from=1700000000000,range=300000,seed=plots[0].tileSeed;
   let gradientCreations=0,dashes=[];
   const ctx={save(){},beginPath(){},rect(){},clip(){},stroke(){},restore(){},moveTo(){},lineTo(){},setLineDash(v){dashes=v},createLinearGradient(x0,y0,x1,y1){gradientCreations++;return {stops:[],addColorStop(at,color){this.stops.push([at,color])}}},fillRect(x,y,w,h){fills.push({x,y,w,h,color:this.fillStyle.stops?JSON.stringify(this.fillStyle.stops):this.fillStyle})}};
   let fills=[];const item={tileSeed:seed},u={ctx,bbox:{left:0,top:0,width:320*ratio,height:600*ratio},valToPos:t=>(t-item.from)/range*600*ratio};
   function capture(start){
    fills=[];item.from=start;item.to=start+range;tileGrid(item,u);return fills.slice(1);
   }
   const a=capture(from),b=capture(from+1000),colors=new Set(a.map(c=>c.color));
   let overlap=0;for(const cell of a){const match=b.find(c=>c.x===cell.x&&Math.abs(c.y-(cell.y-2*ratio))<1e-6);if(match){if(match.color!==cell.color)throw Error('Tile changed shade on scroll');overlap++;}}
   const rows=Array.from({length:128},(_,r)=>Array.from({length:8},(_,c)=>tileShade(r,c,seed)).join(','));
   if(new Set(rows).size<120)throw Error('Short repeating tile pattern');
   const start=performance.now();for(let i=0;i<100;i++)capture(from+i*1000);
   const elapsed=performance.now()-start;
   if(gradientCreations!==7)throw Error('Scrolling recreated cached gradients');
   if(JSON.stringify(dashes)!==JSON.stringify([5*ratio,5*ratio]))throw Error('Grid is not dashed');
   const opaque=[...item.tilePalette.colors.values()].every(g=>g.stops.every(([,c])=>/^#[0-9a-f]{6}$/.test(c)));
   const beforeResize=item.tilePalette;u.bbox.width+=10;capture(from);if(beforeResize===item.tilePalette||gradientCreations!==14)throw Error('Gradient cache did not follow resize');
   const canvas=document.createElement('canvas');canvas.width=640*ratio;canvas.height=600*ratio;const real=canvas.getContext('2d');
   for(let n=0;n<2;n++)tileGrid({from,to:from+range,tileSeed:seed+n},{ctx:real,bbox:{left:n*320*ratio,top:0,width:320*ratio,height:600*ratio},valToPos:t=>(t-from)/range*600*ratio});
   const pixel=(x,y)=>real.getImageData(Math.floor(x*ratio),Math.floor(y*ratio),1,1).data[0];
   const contrasts=[43,83,143,203,263].map(y=>pixel(315,y)-pixel(325,y));
   if(!contrasts.every(d=>d>=6))throw Error('Adjacent track boundary is not visible');
   return {colors:colors.size,overlap,opaque,hundred_draws_ms:elapsed,cached_gradients_during_scroll:7,resize_rebuilds_cache:true,dash_css_pixels:[5,5],adjacent_track_pixel_contrasts:contrasts};
  });
  assert(tiles.colors>=5&&tiles.overlap>100&&tiles.opaque);
  const area=page.locator('.plot').first();await area.hover();
  await page.mouse.wheel(0,-120);
  const motion=[];for(let i=0;i<8;i++){await page.waitForTimeout(45);motion.push(await page.evaluate(()=>offset));}
  assert(motion[0]>0&&motion.at(-1)>motion[0]);assert(new Set(motion.map(v=>v.toFixed(2))).size>=4);
  await page.waitForTimeout(700);assert(await page.evaluate(()=>Math.abs(offset-offsetTarget)<.01));
  const age=await page.evaluate(()=>offset);await page.mouse.wheel(0,120);await page.waitForTimeout(850);assert((await page.evaluate(()=>offset))<age);
  await page.locator('#live').click();await page.waitForTimeout(700);assert.equal(await page.evaluate(()=>offset),0);
  assert(plotRequests.length<15,'Plot requests followed animation frames');
  await page.evaluate(()=>{plots[0].area.dispatchEvent(new WheelEvent('wheel',{deltaY:-120,bubbles:true,cancelable:true}));document.querySelector('#live').click();document.querySelector('[data-tab=calibration]').click();});
  await page.waitForTimeout(100);assert.equal(await page.evaluate(()=>historyAnchor),0);assert.equal(await page.evaluate(()=>offset),0);
  await page.locator('[data-tab="calibration"]').click();await page.waitForSelector('#mapRows tr');
  assert(await page.locator('#calibrateParam').isDisabled());await page.locator('#mapRows tr').first().click();
  await page.locator('#calibrateParam').click();await page.waitForSelector('#calibrationDialog[open]');
  assert.equal(await page.locator('#signalView').inputValue(),'volts');
  const original=await page.evaluate(()=>JSON.stringify(map.parameters[0].points));
  const viewport=()=>page.evaluate(()=>({...calibrationGeometry}));
  const originalView=await viewport(),calibrationCanvas=page.locator('#calibrationCanvas');
  await page.locator('#calibrationZoomIn').click();await page.waitForTimeout(50);
  const zoomed=await viewport();assert(zoomed.x1-zoomed.x0<(originalView.x1-originalView.x0)*.81);
  assert(zoomed.y1-zoomed.y0<(originalView.y1-originalView.y0)*.81);
  await page.locator('#calibrationZoomOut').click();await page.waitForTimeout(50);
  const unzoomed=await viewport();assert(Math.abs((unzoomed.x1-unzoomed.x0)/(originalView.x1-originalView.x0)-1)<1e-10);
  await page.evaluate(()=>{
   const ctx=document.querySelector('#calibrationCanvas').getContext('2d');
   const fill=ctx.fillRect.bind(ctx),move=ctx.moveTo.bind(ctx),text=ctx.fillText.bind(ctx);
   ctx.fillRect=(x,y,w,h)=>{if(x===0&&y===0&&w>100)window.gridCapture={moves:[],labels:[]};return fill(x,y,w,h);};
   ctx.moveTo=(x,y)=>{if(ctx.getLineDash().length)window.gridCapture.moves.push({x,y});return move(x,y);};
   ctx.fillText=(value,x,y)=>{if(ctx.getLineDash().length)window.gridCapture.labels.push({text:String(value),x,y});return text(value,x,y);};
   drawCalibration();
  });
  const gridBefore=await page.evaluate(()=>window.gridCapture);
  const box=await calibrationCanvas.boundingBox(),start={x:box.x+originalView.x+originalView.w*.5,y:box.y+originalView.y+originalView.h*.5};
  await page.mouse.move(start.x,start.y);await page.mouse.down();await page.mouse.move(start.x+65,start.y+35,{steps:8});await page.mouse.up();await page.waitForTimeout(50);
  const panned=await viewport();assert(panned.x0<unzoomed.x0&&panned.y0>unzoomed.y0);
  const gridAfter=await page.evaluate(()=>window.gridCapture);
  for(const axis of ['x','y']){
   const isTick=p=>axis==='x'?p.y>unzoomed.y+unzoomed.h:p.x<unzoomed.x;
   const before=gridBefore.labels.filter(isTick),after=gridAfter.labels.filter(isTick);
   const matches=before.map(a=>({a,b:after.find(b=>a.text===b.text)})).filter(pair=>pair.b);
   assert(matches.length>0,'No shared value-anchored grid ticks after pan');
   for(const {a,b} of matches){assert(Math.abs(b[axis]-a[axis]-(axis==='x'?65:35))<.01,'Grid tick did not follow the curve displacement');
    for(const [capture,label] of [[gridBefore,a],[gridAfter,b]])assert(capture.moves.some(p=>Math.abs(p.x-(axis==='x'?label.x:unzoomed.x))<.01&&Math.abs(p.y-(axis==='x'?unzoomed.y:label.y-5))<.01),'Grid stroke is detached from its label');
   }
  }
  assert(Math.abs((panned.x1-panned.x0)/(unzoomed.x1-unzoomed.x0)-1)<1e-10);
  assert(!await calibrationCanvas.evaluate(e=>e.classList.contains('panning')));
  assert.equal(await page.evaluate(()=>JSON.stringify(calibrationDraft.points)),original);assert(!await page.evaluate(()=>dirty));
  assert.equal(await page.evaluate(async()=>JSON.stringify((await api('calibration')).map.parameters[0].points)),original);
  await page.screenshot({path:path.join(qa,'calibration-panned.png')});
  await page.locator('#calibrationFit').click();await page.waitForTimeout(50);
  const fitted=await viewport();assert.equal(fitted.x0,originalView.x0);assert.equal(fitted.y1,originalView.y1);
  await page.mouse.move(start.x,start.y);await page.mouse.down();
  await page.evaluate(()=>document.querySelector('#calibrationCanvas').dispatchEvent(new PointerEvent('pointercancel',{pointerId:calibrationPan.id})));
  await page.mouse.up();assert.equal(await page.evaluate(()=>calibrationPan),null);
  await page.locator('#calibrationZoomIn').click();await page.selectOption('#signalView','raw');
  assert.equal(await page.evaluate(()=>calibrationDraft.view),null);assert((await viewport()).y1>60000);
  await page.selectOption('#signalView','raw');await page.selectOption('#signalView','volts');
  await page.locator('#cancelCalibration').click();assert.equal(await page.evaluate(()=>JSON.stringify(map.parameters[0].points)),original);
  await page.locator('#calibrateParam').click();
  const field=(row,column)=>page.locator('#pointRows tr').nth(row).locator('input').nth(column);
  await field(0,0).fill('1');await field(0,1).fill('0');await field(1,0).fill('5');await field(1,1).fill('100');
  await page.locator('#addPoint').click();await field(2,0).fill('5');await field(2,1).fill('150');
  await page.locator('#acceptCalibration').click();assert(await page.locator('#calibrationError').textContent());
  await field(2,0).fill('7');await page.screenshot({path:path.join(qa,'calibration-points.png')});
  await page.locator('#acceptCalibration').click();assert.equal(await page.evaluate(()=>revision),1);
  await page.locator('#saveMap').click();await page.waitForFunction(()=>revision===2&&activeRevision===2);
  await page.locator('#calibrateParam').click();assert.equal(Number(await field(0,0).inputValue()),1);assert.equal(Number(await field(1,0).inputValue()),5);assert.equal(Number(await field(2,0).inputValue()),7);
  await page.setViewportSize({width:1280,height:720});await page.waitForTimeout(150);
  assert(await page.locator('#acceptCalibration').evaluate(e=>{const b=e.getBoundingClientRect();return b.bottom<=innerHeight&&b.right<=innerWidth}));
  await page.screenshot({path:path.join(qa,'calibration-1280x720.png')});await page.setViewportSize({width:1440,height:960});
  const downloadPromise=page.waitForEvent('download');await page.locator('#exportPoints').click();const download=await downloadPromise;
  const exported=JSON.parse(fs.readFileSync(await download.path(),'utf8'));assert.equal(exported.format,'kern-calibration-points-v1');assert.equal(exported.points.length,3);
  await page.locator('#pointsFile').setInputFiles({name:'invalid.json',mimeType:'application/json',buffer:Buffer.from(JSON.stringify({...exported,unit:'wrong'}))});
  await page.waitForFunction(()=>document.querySelector('#calibrationError').textContent.length>0);
  await page.locator('#pointsFile').setInputFiles({name:'points.json',mimeType:'application/json',buffer:Buffer.from(JSON.stringify(exported))});
  await page.waitForFunction(()=>document.querySelector('#calibrationError').textContent==='');
  await page.locator('#pointFactor').fill('2');await page.locator('#multiplyPoints').click();assert.equal(Number(await field(1,1).inputValue()),200);
  await page.locator('#cancelCalibration').click();assert.equal(await page.evaluate(()=>map.parameters[0].points[1].value),100);
  await page.locator('#calibrateParam').click();await field(0,0).fill('2');
  await page.evaluate(async()=>{const external=cloneMap(map);external.parameters[1].name='External edit';await api('calibration',{expected_revision:revision,map:external});});
  await page.waitForFunction(()=>activeRevision>revision);
  assert.equal(Number(await field(0,0).inputValue()),2);assert.equal(await page.evaluate(()=>revision),2);
  await page.locator('#cancelCalibration').click();await page.waitForFunction(()=>revision===3);
  await page.locator('#mapRows tr').nth(3).click();await page.locator('#calibrateParam').click();assert.equal(await page.locator('#signalView').inputValue(),'raw');await page.locator('#cancelCalibration').click();
  await page.locator('#mapRows tr').nth(4).click();await page.locator('#calibrateParam').click();assert.equal(await page.locator('#signalView').inputValue(),'hz');await page.locator('#cancelCalibration').click();
  await page.locator('#mapRows tr').first().dblclick();await page.locator('#edit-name').fill('Reference sensor');await page.locator('#acceptEdit').click();await page.waitForFunction(()=>!document.querySelector('#editor').open);
  assert(await page.evaluate(()=>dirty));await page.locator('#undoParam').click();assert.equal(await page.evaluate(()=>parameterDraft.parameters[0].name),await page.evaluate(()=>map.parameters[0].name));
  await page.locator('#reloadMap').click();await page.waitForFunction(()=>!dirty);await page.screenshot({path:path.join(qa,'parameter-settings.png')});
  assert.deepEqual(errors,[]);
  const report={passed:true,tile_rendering:tiles,tests:['stable pseudorandom tile gradients, visible adjacent-track contrast, dashed grid and cached resize/scroll rendering','reversed wheel direction','switching tabs during return to live clears history anchor','multiple intermediate scroll positions; settled target; bounded plot requests','parameter selection and separate calibration window','calibration zoom in/out changes both axes reversibly','calibration background pan preserves points, draft and core map','grid strokes and equal-valued ticks follow both pan axes','fit and signal-unit changes reset viewport','pointer cancellation releases calibration pan','voltage/raw view roundtrip without changing points','duplicate signal rejected','draft then save/activate','voltage points reopen correctly','1280x720 modal controls fit','point export/import with unit validation','coefficient edits isolated by cancel','external map revision preserves open editor; cancel reloads it','counter/frequency units preserved','parameter metadata edit and undo','no browser errors'],scroll_positions:motion.length};
  fs.writeFileSync(path.join(qa,'browser-report.json'),JSON.stringify(report,null,2)+'\n');console.log(JSON.stringify(report));
 }finally{await browser.close()}
})().catch(error=>{console.error(error);process.exit(1)});
