// Requires Playwright as an optional local check dependency; no test runner is used.
const path = require('path');
const { createRequire } = require('module');
const assert = require('assert/strict');
const frontendRequire = createRequire(path.join(__dirname, '../frontend/package.json'));
const { chromium } = process.env.ANPOD_PLAYWRIGHT_MODULE ? require(process.env.ANPOD_PLAYWRIGHT_MODULE) : frontendRequire('playwright');
const fs = require('fs');
const out = path.resolve(__dirname, '../.tmp-frontend-check');
fs.mkdirSync(out, {recursive:true});
(async () => {
 const browser=await chromium.launch({executablePath:process.env.ANPOD_BROWSER_EXECUTABLE || undefined,headless:true});
 const page=await browser.newPage({viewport:{width:420,height:590}});
 const errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(() => {
  let id=0;const callbacks=new Map(),listeners=new Map();
  const device={serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true};
  let snapshot={state:'noDevice',message:'尚未连接手机',deviceName:null,devices:[],selectedSerial:null,transport:'usb',output:'computer',settings:{schemaVersion:1,autoConnectLastDevice:true,lastDeviceId:null,minimizeToTray:true,launchAtLogin:false,preferredTransport:'Usb',autoReconnectWifi:true,lastWirelessEndpoint:null,playbackMode:'Balanced',appearanceTheme:'System'},busy:false,canDisconnect:false,canChangeModes:true,statusDetail:'声音继续从电脑播放。',version:'1.0.3'};
  const emit=()=>{for(const [key,cb]of listeners)if(key==='anpod://snapshot')callbacks.get(cb)?.({event:key,id:cb,payload:structuredClone(snapshot)});};
  window.reviewSet=(state,transport='usb')=>{snapshot={...snapshot,state,transport,devices:state==='noDevice'?[]:[{...device,transport}],selectedSerial:state==='noDevice'?null:device.serial,deviceName:state==='noDevice'?null:device.displayName,canDisconnect:['connecting','connected','streaming','reconnecting'].includes(state),busy:['connecting','reconnecting'].includes(state),output:state==='streaming'?'phone':'computer'};emit();};
  window.reviewEmit=emit;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener() {}};
  window.__TAURI_INTERNALS__={transformCallback(fn){callbacks.set(++id,fn);return id;},unregisterCallback(cb){callbacks.delete(cb);},metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},async invoke(cmd,args){
   if(cmd==='plugin:event|listen'){listeners.set(args.event,args.handler);return ++id;}
   if(cmd==='plugin:event|unlisten')return;
   if(cmd==='qr_credentials')return {instanceName:'studio-Test123456',password:'abcdefghijklmnopqrst',payload:'WIFI:T:ADB;S:studio-Test123456;P:abcdefghijklmnopqrst;;'};
   if(cmd==='pair_wifi')return new Promise(resolve=>{window.reviewCompletePair=()=>resolve({success:true,message:'已配对',snapshot:structuredClone(snapshot)});});
   if(cmd==='update_setting'){snapshot.settings[args.key]=args.value;return structuredClone(snapshot);}
   if(cmd==='connect_device'){window.reviewSet('streaming',snapshot.transport);return {success:true,message:'已连接',snapshot:structuredClone(snapshot)};}
   if(cmd==='disconnect'){window.reviewSet('ready',snapshot.transport);return {success:true,message:'已断开',snapshot:structuredClone(snapshot)};}
   if(cmd==='set_output'){snapshot.output=args.output;snapshot.state=args.output==='phone'?'streaming':'connected';emit();}
   if(cmd==='reconfigure_session'){snapshot.settings[args.kind==='playback'?'playbackMode':'preferredTransport']=args.kind==='playback'?(args.value==='balanced'?'Balanced':'LowLatency'):(args.value==='usb'?'Usb':'Wifi');snapshot.transport=snapshot.settings.preferredTransport==='Wifi'?'wifi':'usb';window.reviewSet(snapshot.state,snapshot.transport);}
   return structuredClone(snapshot);
  }};
 });
 await page.goto(process.env.ANPOD_UI_URL || 'http://127.0.0.1:1420');
 await page.locator('.app-shell').waitFor();
 const results={errors,layouts:[],focusStable:null};
 for(const size of [[420,590],[360,500]]){
  await page.setViewportSize({width:size[0],height:size[1]});
  for(const theme of ['light','dark']){
   await page.emulateMedia({colorScheme:theme,reducedMotion:'reduce'});
   for(const state of ['noDevice','ready','streaming','connecting','reconnecting','error']){
    await page.evaluate(s=>window.reviewSet(s),state);await page.waitForTimeout(80);
    const measure=await page.evaluate(()=>{const shell=document.querySelector('.app-shell'),dev=document.querySelector('.device-select'),copy=document.querySelector('.device-copy');return {width:shell.getBoundingClientRect().width,horizontalOverflow:document.documentElement.scrollWidth>innerWidth,deviceHeight:dev.getBoundingClientRect().height,deviceTextHeight:copy.getBoundingClientRect().height};});
    results.layouts.push({size,theme,state,...measure});
    if(size[0]===420&&theme==='light'&&['noDevice','streaming'].includes(state))await page.screenshot({path:`${out}/${state}.png`});
   }
  }
 }
 await page.setViewportSize({width:420,height:590});await page.evaluate(()=>window.reviewSet('ready','wifi'));
 await page.getByRole('button',{name:'添加手机',exact:true}).click();
 await page.getByRole('tab',{name:'配对码',exact:true}).click();
 const code=page.getByLabel('6 位配对码');await code.fill('123456');await code.focus();
 await page.evaluate(()=>window.reviewEmit());await page.waitForTimeout(100);
 results.focusStable=await code.evaluate(el=>document.activeElement===el);
 results.codeRetained=await code.inputValue();
 await page.screenshot({path:`${out}/pair-code.png`});
 await page.getByLabel('手机配对地址').fill('192.0.2.20:37001');
 await page.getByRole('button',{name:'配对并添加',exact:true}).click();
 await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
 await page.evaluate(()=>window.reviewCompletePair());await page.waitForTimeout(200);
 results.stalePairReopens=await page.locator('dialog[open]').count()>0;
 if(results.stalePairReopens)await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
 await page.evaluate(()=>window.reviewSet('ready','usb'));
 await page.locator('.output-card').filter({hasText:'手机'}).click();
 await page.getByRole('button',{name:'断开连接',exact:true}).waitFor();
 results.disconnectEnabled=await page.getByRole('button',{name:'断开连接',exact:true}).isEnabled();
 results.deviceLocked=await page.locator('.device-select').isDisabled();
 await page.getByRole('button',{name:'低延迟',exact:true}).click();
 results.modeNeedsConfirmation=await page.locator('dialog[open]').count()===1;
 await page.getByRole('button',{name:'取消',exact:true}).click();
 results.cancelKeepsMode=await page.getByRole('button',{name:'均衡',exact:true}).getAttribute('aria-pressed')==='true';
 await page.getByRole('button',{name:'低延迟',exact:true}).click();
 await page.getByRole('button',{name:'断开并重新连接',exact:true}).click();
 results.modeSwitchWorks=await page.getByRole('button',{name:'低延迟',exact:true}).getAttribute('aria-pressed')==='true';
 await page.getByRole('button',{name:'Wi-Fi',exact:true}).click();
 await page.getByRole('button',{name:'断开并重新连接',exact:true}).click();
 results.transportSwitchWorks=await page.getByRole('button',{name:'Wi-Fi',exact:true}).getAttribute('aria-pressed')==='true';
 await page.getByRole('button',{name:'断开连接',exact:true}).click();
 results.disconnectWorks=await page.getByRole('button',{name:'断开连接',exact:true}).count()===0;
 await page.getByRole('button',{name:'打开设置',exact:true}).click();
 await page.getByRole('checkbox',{name:/放大文字/}).check();
 await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
 await page.setViewportSize({width:360,height:500});await page.emulateMedia({colorScheme:'dark'});
 results.largeTextNoOverflow=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth && document.querySelector('.device-select').getBoundingClientRect().height>=document.querySelector('.device-copy').getBoundingClientRect().height);
 await page.screenshot({path:`${out}/large-dark.png`});
 fs.writeFileSync(`${out}/result.json`,JSON.stringify(results,null,2));
 const {layouts,...summary}=results;
 console.log(JSON.stringify({...summary,layoutFailures:layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight)}));
 await browser.close();
 assert.deepEqual(errors, []);
 assert.equal(layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight||x.width!==x.size[0]).length, 0);
 assert.equal(results.stalePairReopens, false);
 for(const key of ['focusStable','disconnectEnabled','deviceLocked','modeNeedsConfirmation','cancelKeepsMode','modeSwitchWorks','transportSwitchWorks','disconnectWorks','largeTextNoOverflow'])assert.equal(results[key], true, key);
 assert.equal(results.codeRetained, '123456');
})().catch(e=>{console.error(e);process.exit(1)});
