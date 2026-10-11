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
  const secondDevice={...device,serial:'TESTSECOND',transport:'usb',displayName:'Long Google Pixel Wireless Device Name',androidVersion:'14'};
  let snapshot={state:'noDevice',message:'尚未连接手机',deviceName:null,devices:[],selectedSerial:null,sessionSerial:null,sessionTransport:null,sessionAndroidVersion:null,transport:'usb',output:'computer',settings:{schemaVersion:1,autoConnectLastDevice:true,lastDeviceId:null,minimizeToTray:true,launchAtLogin:false,autoReconnectWifi:true,preferredTransport:'Usb',lastWirelessEndpoint:null,playbackMode:'Balanced',appearanceTheme:'System'},busy:false,canDisconnect:false,canChangeModes:true,statusDetail:'声音继续从电脑播放。',version:'1.0.3'};
  window.reviewCalls={};
  const emit=()=>{for(const [key,cb]of listeners)if(key==='anpod://snapshot')callbacks.get(cb)?.({event:key,id:cb,payload:structuredClone(snapshot)});};
  window.reviewRead=()=>structuredClone(snapshot);
  window.reviewSet=(state,transport='usb',serial='TESTUSB')=>{const all=state==='noDevice'?[]:[{...device,transport},{...secondDevice,transport}];const active=['connected','streaming','reconnecting'].includes(state);const current=all.find(d=>d.serial===serial);snapshot={...snapshot,state,transport,devices:all,selectedSerial:active?serial:(snapshot.selectedSerial??serial),sessionSerial:active?serial:null,sessionTransport:active?transport:null,sessionAndroidVersion:active?current?.androidVersion??null:null,deviceName:active?current?.displayName:state==='error'?'Google Pixel 8 Pro':null,canDisconnect:['connecting','connected','streaming','reconnecting','error'].includes(state),busy:['connecting','reconnecting'].includes(state),output:state==='streaming'?'phone':'computer'};emit();};
  window.reviewHideSessionDevice=()=>{snapshot.devices=snapshot.devices.filter(device=>device.serial!==snapshot.sessionSerial);emit();};
  window.reviewEmit=emit;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener() {}};
  window.__TAURI_INTERNALS__={transformCallback(fn){callbacks.set(++id,fn);return id;},unregisterCallback(cb){callbacks.delete(cb);},metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},async invoke(cmd,args){
   window.reviewCalls[cmd]=(window.reviewCalls[cmd]||0)+1;
   if(cmd==='plugin:event|listen'){listeners.set(args.event,args.handler);return ++id;}
   if(cmd==='plugin:event|unlisten')return;
   if(cmd==='qr_credentials')return {instanceName:'studio-Test123456',password:'abcdefghijklmnopqrst',payload:'WIFI:T:ADB;S:studio-Test123456;P:abcdefghijklmnopqrst;;'};
   if(cmd==='pair_wifi')return new Promise(resolve=>{window.reviewCompletePair=()=>resolve({success:true,message:'已配对',snapshot:structuredClone(snapshot)});});
   if(cmd==='update_setting'){snapshot.settings[args.key]=args.value;return structuredClone(snapshot);}
   if(cmd==='connect_device'){window.reviewSet('streaming',snapshot.transport);return {success:true,message:'已连接',snapshot:structuredClone(snapshot)};}
   if(cmd==='connect_device_only'){window.reviewSet('connected',snapshot.transport,args.serial);return {success:true,message:'已连接',snapshot:structuredClone(snapshot)};}
   if(cmd==='switch_device'){if(window.reviewSwitchMode==='failure')return {success:false,message:'模拟切换失败，已恢复原连接',snapshot:structuredClone(snapshot)};window.reviewSet(snapshot.output==='phone'?'streaming':'connected',snapshot.transport,args.serial);snapshot.selectedSerial=args.serial;return {success:true,message:'已切换',snapshot:structuredClone(snapshot)};}
   if(cmd==='select_device'){snapshot.selectedSerial=args.serial;return structuredClone(snapshot);}
   if(cmd==='cancel_operation'){window.reviewSet('ready',snapshot.transport);return structuredClone(snapshot);}
   if(cmd==='disconnect'){window.reviewSet('ready',snapshot.transport);return {success:true,message:'已断开',snapshot:structuredClone(snapshot)};}
   if(cmd==='set_output'){snapshot.output=args.output;snapshot.state=args.output==='phone'?'streaming':'connected';emit();return structuredClone(snapshot);}
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
    const measure=await page.evaluate(()=>{const shell=document.querySelector('.app-shell'),dev=document.querySelector('.device-card'),copy=document.querySelector('.device-copy');return {width:shell.getBoundingClientRect().width,horizontalOverflow:document.documentElement.scrollWidth>innerWidth,deviceHeight:dev.getBoundingClientRect().height,deviceTextHeight:copy.getBoundingClientRect().height};});
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
 await page.evaluate(()=>window.reviewSet('error','usb'));
 results.errorCleanupKeepsAction=await page.evaluate(()=>window.reviewRead().canDisconnect && !window.reviewRead().sessionSerial) && await page.locator('.device-action').innerText()==='断开连接';
 await page.locator('.device-action').click();
 results.errorCleanupRetryWorks=await page.evaluate(()=>window.reviewCalls.disconnect===1 && !window.reviewRead().canDisconnect);
 const connectOnlyCalls=await page.evaluate(()=>window.reviewCalls.connect_device_only||0);
 await page.locator('.device-select').click();
 results.deviceMenuOpens=await page.locator('.device-option').count()===2 && await page.locator('.device-select').getAttribute('aria-expanded')==='true';
 await page.locator('.device-option').nth(1).click();
 results.menuSelectionNoConnection=await page.evaluate(()=>window.reviewCalls.connect_device_only||0)===connectOnlyCalls && await page.evaluate(()=>window.reviewCalls.connect_device||0)===0;
 results.idleDeviceSelected=await page.evaluate(()=>window.reviewRead().selectedSerial==='TESTSECOND' && !window.reviewRead().sessionSerial);
 await page.locator('.device-action').click();
 await page.getByRole('button',{name:'断开连接',exact:true}).waitFor();
 results.connectOnlyKeepsComputerOutput=await page.evaluate(()=>window.reviewCalls.connect_device_only===1 && (window.reviewCalls.connect_device||0)===0 && window.reviewRead().output==='computer' && window.reviewRead().state==='connected' && document.querySelector('.device-card').textContent.includes('USB · ADB') && document.querySelector('.device-card').textContent.includes('Android 14'));
 await page.locator('.output-card').filter({hasText:'手机'}).click();
 results.outputCardStartsPlayback=await page.evaluate(()=>window.reviewRead().output==='phone' && window.reviewRead().state==='streaming');
 results.connectedDeviceCardExpandable=await page.locator('.device-select').isEnabled();
 await page.locator('.device-select').click();
 results.activeDeviceMarked=await page.locator('.device-option').nth(1).getAttribute('aria-pressed')==='true' && await page.locator('.device-option').nth(1).innerText().then(text=>text.includes('当前连接'));
 await page.locator('.device-option').nth(0).click();
 results.switchNeedsConfirmation=await page.getByRole('dialog',{name:'切换连接手机？'}).count()===1;
 await page.getByRole('button',{name:'取消',exact:true}).click();
 results.cancelSwitchKeepsDeviceAndOutput=await page.evaluate(()=>window.reviewCalls.switch_device||0)===0 && await page.evaluate(()=>window.reviewRead().sessionSerial==='TESTSECOND' && window.reviewRead().output==='phone');
 await page.locator('.device-select').click();await page.locator('.device-option').nth(0).click();
 await page.evaluate(()=>window.reviewSwitchMode='failure');
 await page.getByRole('button',{name:'中断并切换',exact:true}).click();
 await page.waitForTimeout(100);
 results.failedSwitchKeepsDeviceAndOutput=await page.locator('.device-card').innerText().then(text=>text.includes('Long Google Pixel Wireless Device Name') && text.includes('正在手机播放'));
 await page.evaluate(()=>window.reviewSwitchMode='success');
 await page.locator('.device-select').click();await page.locator('.device-option').nth(0).click();
 await page.getByRole('button',{name:'中断并切换',exact:true}).click();
 results.switchPreservesPhoneOutput=await page.locator('.device-card').innerText().then(text=>text.includes('Google Pixel 8 Pro') && text.includes('正在手机播放'));
 await page.evaluate(()=>window.reviewHideSessionDevice());
 results.missingDeviceScanUsesSessionDetails=await page.locator('.device-card').innerText().then(text=>text.includes('Google Pixel 8 Pro') && text.includes('USB · ADB') && !text.includes('Long Google Pixel Wireless Device Name'));
 await page.locator('.device-select').click();
 results.missingDeviceScanShowsCurrentMenuItem=await page.locator('.device-option').first().innerText().then(text=>text.includes('当前连接')) && await page.locator('.device-option').count()===2;
 const switchesBeforeSame=await page.evaluate(()=>window.reviewCalls.switch_device||0);
 await page.locator('.device-option').first().click();
 results.sameDeviceSelectionHasNoSideEffect=await page.evaluate(n=>window.reviewCalls.switch_device===n, switchesBeforeSame) && await page.locator('dialog[open]').count()===0;
 await page.evaluate(()=>window.reviewSet('streaming','usb','TESTUSB'));
 await page.getByRole('button',{name:'Wi-Fi',exact:true}).click();
 await page.getByRole('button',{name:'断开并重新连接',exact:true}).click();
 results.addPhoneAvailableDuringSession=await page.getByRole('button',{name:'添加手机',exact:true}).isEnabled();
 await page.getByRole('button',{name:'添加手机',exact:true}).click();
 results.addPhoneOpensDuringSession=await page.getByRole('dialog',{name:'添加无线手机'}).count()===1;
 await page.evaluate(()=>window.reviewSet('reconnecting','wifi'));
 results.busyBlocksPairing=await page.getByRole('tab',{name:'配对码',exact:true}).isDisabled() && await page.getByRole('button',{name:'开始扫码配对',exact:true}).isDisabled();
 await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
 await page.evaluate(()=>window.reviewSet('streaming','wifi','TESTUSB'));
 await page.locator('.device-select').click();
 await page.evaluate(()=>window.reviewSet('reconnecting','wifi'));
 results.busyBlocksDeviceChoices=await page.locator('.device-option').nth(1).isDisabled();
 await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
 results.busyBlocksAddAndModes=await page.getByRole('button',{name:'添加手机',exact:true}).isDisabled() && await page.getByRole('button',{name:'低延迟',exact:true}).isDisabled() && await page.getByRole('button',{name:'Wi-Fi',exact:true}).isDisabled();
 await page.evaluate(()=>window.reviewSet('connecting','wifi'));
 results.connectingCanBeCancelled=await page.getByRole('button',{name:'取消连接',exact:true}).isEnabled();
 await page.getByRole('button',{name:'取消连接',exact:true}).click();
 results.cancelConnectionWorks=await page.evaluate(()=>window.reviewCalls.cancel_operation===1) && await page.locator('.device-action').isEnabled();
 await page.locator('.output-card').filter({hasText:'手机'}).click();
 await page.getByRole('button',{name:'低延迟',exact:true}).waitFor();
 results.disconnectEnabled=await page.getByRole('button',{name:'断开连接',exact:true}).isEnabled();
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
 results.largeTextNoOverflow=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth && document.querySelector('.device-card').getBoundingClientRect().height>=document.querySelector('.device-copy').getBoundingClientRect().height);
 await page.screenshot({path:`${out}/large-dark.png`});
 fs.writeFileSync(`${out}/result.json`,JSON.stringify(results,null,2));
 const {layouts,...summary}=results;
 console.log(JSON.stringify({...summary,layoutFailures:layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight)}));
 await browser.close();
 assert.deepEqual(errors, []);
 assert.equal(layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight||x.width!==x.size[0]).length, 0);
 assert.equal(results.stalePairReopens, false);
 for(const key of ['focusStable','errorCleanupKeepsAction','errorCleanupRetryWorks','deviceMenuOpens','menuSelectionNoConnection','idleDeviceSelected','connectOnlyKeepsComputerOutput','outputCardStartsPlayback','connectedDeviceCardExpandable','activeDeviceMarked','switchNeedsConfirmation','cancelSwitchKeepsDeviceAndOutput','failedSwitchKeepsDeviceAndOutput','switchPreservesPhoneOutput','missingDeviceScanUsesSessionDetails','missingDeviceScanShowsCurrentMenuItem','sameDeviceSelectionHasNoSideEffect','addPhoneAvailableDuringSession','addPhoneOpensDuringSession','busyBlocksPairing','busyBlocksDeviceChoices','busyBlocksAddAndModes','connectingCanBeCancelled','cancelConnectionWorks','disconnectEnabled','modeNeedsConfirmation','cancelKeepsMode','modeSwitchWorks','transportSwitchWorks','disconnectWorks','largeTextNoOverflow'])assert.equal(results[key], true, key);
 assert.equal(results.codeRetained, '123456');
})().catch(e=>{console.error(e);process.exit(1)});
