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
  let reviewInterfaces=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}];let reviewReceiverInterfaces=[{interfaceIndex:1,interfaceName:'Wi-Fi',address:'192.168.50.20',prefixLength:24}];let reviewBindings=['phoneA-abcdef','phoneB-abcdef'];let reviewCandidates=[{endpoint:'192.168.50.3:45678'}];let reviewOfferAddress=null;let reviewListenerError='';
  let snapshot={state:'noDevice',message:'尚未连接手机',deviceName:null,devices:[],selectedSerial:null,sessionSerial:null,sessionReceiverBinding:null,sessionTransport:null,sessionConnectionTransport:null,sessionWirelessProtocol:null,sessionCapabilities:null,sessionAndroidVersion:null,playbackDiagnostics:null,wirelessConnectionOffers:{active:false,automaticBinding:null,interfaceAddress:null,interfacePrefixLength:null,offers:[]},transport:'usb',connectionTransport:'usb',output:'computer',audioBitrateBps:null,settings:{schemaVersion:1,autoConnectLastDevice:true,lastDeviceId:null,minimizeToTray:true,launchAtLogin:false,autoReconnectWifi:true,preferredTransport:'Usb',wirelessProtocol:'udp',audioBackend:'receiver',lastWirelessEndpoint:null,playbackMode:'Balanced',appearanceTheme:'System'},busy:false,canDisconnect:false,canChangeModes:true,statusDetail:'声音继续从电脑播放。',version:'1.0.3'};
  window.reviewCalls={};window.reviewCommandArgs={};
  window.reviewGetUsbStatus=serial=>structuredClone(reviewUsbStatuses[serial]);
  const emit=()=>{for(const [key,cb]of listeners)if(key==='anpod://snapshot')callbacks.get(cb)?.({event:key,id:cb,payload:structuredClone(snapshot)});};
  window.reviewRead=()=>structuredClone(snapshot);
  let reviewCandidate={versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null};
  let reviewUsbStatuses={TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}};
  let reviewUpgradeError='';let reviewInstallError='';let reviewTargetBindings={TESTUSB:'phoneA-abcdef'};
  window.reviewUsbScenario=(options={})=>{
   window.reviewCalls={};window.reviewCommandArgs={};
   reviewCandidate=options.candidate===undefined?{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null}:options.candidate;
   reviewUsbStatuses=options.statuses||{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}};
   reviewBindings=options.bindings===undefined?['phoneA-abcdef']:options.bindings;
   reviewInterfaces=options.pcInterfaces||[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}];
   reviewReceiverInterfaces=options.receiverInterfaces||[{interfaceIndex:1,interfaceName:'Wi-Fi',address:'192.168.50.20',prefixLength:24}];
   reviewUpgradeError=options.upgradeError||'';reviewInstallError=options.installError||'';reviewTargetBindings=options.targetBindings||{TESTUSB:'phoneA-abcdef'};
   snapshot={...snapshot,state:options.active?'streaming':'ready',devices:options.devices||[device],selectedSerial:null,sessionSerial:options.active?(options.sessionSerial||null):null,sessionReceiverBinding:options.active?(options.binding||'phoneA-abcdef'):null,sessionTransport:options.active?(options.sessionSerial?'usb':'lan'):null,sessionConnectionTransport:options.active?(options.sessionSerial?'usb':'wireless'):null,sessionWirelessProtocol:options.active&&!options.sessionSerial?'udp':null,sessionCapabilities:options.active&&!options.sessionSerial?{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:0,versionCode:options.activeVersion||8}:null,canDisconnect:!!options.active,output:options.active?'phone':'computer',busy:false,settings:{...snapshot.settings,preferredTransport:options.active&&!options.sessionSerial?'Wifi':'Usb',wirelessProtocol:'udp'}};emit();
  };
  window.reviewWirelessFlowConfigure=(interfaces,bindings,candidates)=>{reviewInterfaces=structuredClone(interfaces);reviewBindings=[...bindings];reviewCandidates=structuredClone(candidates);reviewOfferAddress=null;reviewListenerError='';snapshot.wirelessConnectionOffers={active:false,automaticBinding:null,interfaceAddress:null,interfacePrefixLength:null,offers:[]};emit();};
  window.reviewWirelessOffers=(offers,active=true,automaticBinding=snapshot.wirelessConnectionOffers.automaticBinding)=>{snapshot.wirelessConnectionOffers={active,automaticBinding,interfaceAddress:active?reviewOfferAddress:null,interfacePrefixLength:active?24:null,offers};emit();};
  window.reviewFailNextListener=(message)=>{reviewListenerError=message;};
  window.reviewAcceptedOffers=[];window.reviewListenerCalls=[];
  window.reviewDiagnosticsScenario=(scenario)=>{
   const udp=scenario==='udp'||scenario==='paused'||scenario==='oldUdp';
   const active=scenario!=='noSession';
   const values={sampleRate:48000,channels:2,receiverSampleRate:udp?44100:48000,receiverChannels:2,pcDroppedFrames:0,pcQueuedFrames:null,udpJitterTargetFrames:udp?480:null,udpExpiredFrames:udp?0:null,udpQueuedFrames:udp?96:null,tcpDroppedBacklogFrames:null,trackPendingFrames:192,underrunCount:0,underrunIncrement:null,playbackAdvancing:scenario==='paused'?false:true,requestedBufferFrames:192,acceptedBufferFrames:240,effectiveBufferFrames:256,capacityFrames:768,startThresholdFrames:192,preRollFrames:96};
    snapshot={...snapshot,state:scenario==='noSession'?'noDevice':scenario==='paused'?'paused':scenario==='usb'||scenario==='udp'||scenario==='oldUdp'?'streaming':'connected',message:active?'已连接 · 手机播放':'尚未连接',deviceName:active?'测试接收端':null,devices:[],selectedSerial:null,sessionSerial:active&&!udp?'TESTUSB':null,sessionReceiverBinding:active&&udp?'phoneA-abcdef':null,sessionTransport:active?(udp?'lan':'usb'):null,sessionConnectionTransport:active?(udp?'wireless':'usb'):null,sessionWirelessProtocol:udp?'udp':null,sessionCapabilities:udp?{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:scenario==='oldUdp'?4:12,versionCode:7}:null,playbackDiagnostics:scenario==='noSession'||scenario==='oldUdp'?null:values,connectionTransport:udp?'wireless':'usb',transport:udp?'wifi':'usb',output:scenario==='paused'?'phone':active?'phone':'computer',canDisconnect:active,busy:false,statusDetail:'模拟诊断快照',settings:{...snapshot.settings,preferredTransport:udp?'Wifi':'Usb',wirelessProtocol:'udp'}};emit();
  };
  window.reviewSetUdpReady=()=>{window.reviewSet('ready','wifi');snapshot.settings.wirelessProtocol='udp';snapshot.connectionTransport='wireless';snapshot.transport='wifi';emit();};
  window.reviewUsbWirelessReady=()=>{snapshot={...snapshot,state:'ready',devices:[device],transport:'wifi',connectionTransport:'wireless',output:'computer',settings:{...snapshot.settings,preferredTransport:'Wifi',wirelessProtocol:'udp'}};emit();};
  window.reviewSet=(state,transport='usb',serial='TESTUSB')=>{const all=state==='noDevice'?[]:[{...device,transport},{...secondDevice,transport}];const active=['connected','streaming','reconnecting'].includes(state);const current=all.find(d=>d.serial===serial);const protocol=transport==='wifi'?'adb':snapshot.settings.wirelessProtocol;snapshot={...snapshot,state,transport,connectionTransport:transport==='wifi'?'wireless':'usb',settings:{...snapshot.settings,preferredTransport:transport==='wifi'?'Wifi':'Usb',wirelessProtocol:protocol},devices:all,selectedSerial:active?serial:(snapshot.selectedSerial??serial),sessionSerial:active?serial:null,sessionReceiverBinding:null,sessionTransport:active?transport:null,sessionConnectionTransport:active?(transport==='wifi'?'wireless':'usb'):null,sessionWirelessProtocol:active?protocol:null,sessionCapabilities:null,sessionAndroidVersion:active?current?.androidVersion??null:null,deviceName:active?current?.displayName:state==='error'?'Google Pixel 8 Pro':null,canDisconnect:['connecting','connected','streaming','reconnecting','error'].includes(state),busy:['connecting','reconnecting'].includes(state),output:state==='streaming'?'phone':'computer'};emit();};
  window.reviewHideSessionDevice=()=>{snapshot.devices=snapshot.devices.filter(device=>device.serial!==snapshot.sessionSerial);emit();};
  window.reviewEmit=emit;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener() {}};
  window.__TAURI_INTERNALS__={transformCallback(fn){callbacks.set(++id,fn);return id;},unregisterCallback(cb){callbacks.delete(cb);},metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},async invoke(cmd,args){
   window.reviewCalls[cmd]=(window.reviewCalls[cmd]||0)+1;window.reviewCommandArgs[cmd]=args;
   if(cmd==='plugin:event|listen'){listeners.set(args.event,args.handler);return ++id;}
   if(cmd==='plugin:event|unlisten')return;
    if(cmd==='receiver_update_candidate')return structuredClone(reviewCandidate);
    if(cmd==='receiver_status'){const status=reviewUsbStatuses[args.serial];if(!status)throw new Error('没有找到已授权的 USB 设备');return structuredClone(status);}
   if(cmd==='receiver_network_interfaces')return structuredClone(reviewReceiverInterfaces);
    if(cmd==='receiver_install'){if(reviewInstallError)throw new Error(reviewInstallError);reviewUsbStatuses[args.serial]={installed:true,versionName:reviewCandidate.versionName,versionCode:reviewCandidate.versionCode,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'};snapshot.state='ready';snapshot.sessionSerial=null;snapshot.sessionReceiverBinding=null;snapshot.canDisconnect=false;snapshot.output='computer';emit();return structuredClone(reviewUsbStatuses[args.serial]);}
    if(cmd==='receiver_upgrade_usb'){if(reviewUpgradeError)throw new Error(reviewUpgradeError);if(reviewTargetBindings[args.serial]!==args.bindingReference)throw new Error('USB 设备序列号或 Android 用户与所选 Receiver 绑定不匹配；未执行升级。');reviewUsbStatuses[args.serial]={installed:true,versionName:reviewCandidate.versionName,versionCode:reviewCandidate.versionCode,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已升级'};snapshot.state='ready';snapshot.sessionSerial=null;snapshot.sessionReceiverBinding=null;snapshot.canDisconnect=false;snapshot.output='computer';emit();return structuredClone(reviewUsbStatuses[args.serial]);}
    if(cmd==='refresh_devices')return structuredClone(snapshot);
   if(cmd==='receiver_binding_references')return [...reviewBindings];
   if(cmd==='network_interfaces')return structuredClone(reviewInterfaces);
   if(cmd==='start_network_audio'){snapshot={...snapshot,state:'connected',message:'首次绑定连接已认证',deviceName:'Google Pixel 8 Pro',sessionSerial:null,sessionReceiverBinding:'new-phone-binding',sessionTransport:'lan',sessionConnectionTransport:'wireless',sessionWirelessProtocol:'udp',sessionCapabilities:{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:12,versionCode:reviewUsbStatuses[args.bootstrapSerial]?.versionCode??null},canDisconnect:true,connectionTransport:'wireless',transport:'wifi',output:'computer',settings:{...snapshot.settings,preferredTransport:'Wifi',wirelessProtocol:'udp'}};emit();return structuredClone(snapshot);}
   if(cmd==='get_snapshot')return structuredClone(snapshot);
   if(cmd==='set_wireless_connection_offers'){window.reviewListenerCalls.push({localAddress:args.localAddress,allowed:args.allowed,bindingReference:args.bindingReference});if(args.allowed&&reviewListenerError){const message=reviewListenerError;reviewListenerError='';throw new Error(message);}if(args.allowed)reviewOfferAddress=args.localAddress;snapshot.wirelessConnectionOffers={active:args.allowed,automaticBinding:args.allowed?(args.bindingReference??(reviewBindings.length===1?reviewBindings[0]:null)):null,interfaceAddress:args.allowed?args.localAddress:null,interfacePrefixLength:args.allowed?24:null,offers:[]};emit();return structuredClone(snapshot.wirelessConnectionOffers);}
   if(cmd==='accept_wireless_connection_offer'){window.reviewAcceptedOffers.push({offerId:args.offerId,bindingReference:args.bindingReference});snapshot.wirelessConnectionOffers={active:false,automaticBinding:null,interfaceAddress:null,interfacePrefixLength:null,offers:[]};snapshot.state='connected';snapshot.message='已认证连接';snapshot.sessionSerial=null;snapshot.sessionReceiverBinding=args.bindingReference;snapshot.sessionTransport='lan';snapshot.sessionConnectionTransport='wireless';snapshot.sessionWirelessProtocol='udp';snapshot.sessionCapabilities={controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:12,versionCode:7};snapshot.canDisconnect=true;emit();return structuredClone(snapshot);}
   if(cmd==='discover_network_receivers')return structuredClone(reviewCandidates);
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
 if(process.env.ANPOD_DIAGNOSTICS_ONLY){
  await page.setViewportSize({width:360,height:640});
  await page.emulateMedia({colorScheme:'light',reducedMotion:'reduce'});
  const openDiagnostics=async()=>page.getByRole('button',{name:/诊断/}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('usb'));
  await openDiagnostics();
  const dialog=page.getByRole('dialog',{name:'诊断'});
  results.usbFormatAndUnknownValues=await dialog.innerText().then(text=>text.includes('PC 采样率')&&text.includes('48,000 Hz')&&text.includes('Receiver 采样率')&&text.includes('PC 当前队列')&&text.includes('未提供')&&text.includes('PC 丢弃')&&text.includes('0 帧 · 会话累计'));
  results.bufferSemanticsVisible=await dialog.innerText().then(text=>['请求','系统接受','当前有效','容量','启动阈值','预填充','AudioTrack'].every(value=>text.includes(value)));
  await page.keyboard.press('Tab');
  results.modalKeyboardFocusStaysInside=await dialog.evaluate(el=>el.contains(document.activeElement));
  results.compactMetricsNoHorizontalOverflow=await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth&&[...el.querySelectorAll('.diagnostic-group dl')].every(dl=>dl.scrollWidth<=dl.clientWidth));
  const populatedModalGeometry=await dialog.evaluate(el=>({width:el.getBoundingClientRect().width,height:el.getBoundingClientRect().height,scrollHeight:el.scrollHeight,scrollWidth:el.scrollWidth}));
  await dialog.evaluate(el=>el.scrollTop=0);
  await page.screenshot({path:`${out}/receiver-diagnostics-compact.png`});
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('udp'));
  await openDiagnostics();
  results.udpFormatAndQueueFields=await dialog.innerText().then(text=>text.includes('Receiver 采样率')&&text.includes('44,100 Hz')&&text.includes('UDP 抖动目标')&&text.includes('480 帧 · 目标队列')&&text.includes('UDP 过期')&&text.includes('0 帧 · 会话累计')&&text.includes('TCP 丢弃积压'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('paused'));
  await openDiagnostics();
  results.pausedSnapshotNotReportedAsPlaying=await dialog.locator('.diagnostic-context').innerText().then(text=>text.includes('手机播放已暂停'))&&await dialog.getByText('播放推进',{exact:true}).locator('..').innerText().then(text=>text.includes('否'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('oldUdp'));
  await openDiagnostics();
  results.oldUdpExplainsMissingDiagnostics=await dialog.locator('.diagnostic-empty').innerText().then(text=>text.includes('未提供播放诊断能力')&&text.includes('可能为旧版本'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('noSession'));
  await openDiagnostics();
  results.noSessionExplainsMissingSnapshot=await dialog.locator('.diagnostic-empty').innerText().then(text=>text.includes('当前没有手机会话'));
  results.compactNoHorizontalOverflow=await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth&&[...el.querySelectorAll('.diagnostic-group dl')].every(dl=>dl.scrollWidth<=dl.clientWidth));
  const modalGeometry=await dialog.evaluate(el=>({width:el.getBoundingClientRect().width,height:el.getBoundingClientRect().height,scrollHeight:el.scrollHeight,scrollWidth:el.scrollWidth}));
  fs.writeFileSync(`${out}/diagnostics-result.json`,JSON.stringify({...results,populatedModalGeometry},null,2));
  console.log(JSON.stringify({usbFormatAndUnknownValues:results.usbFormatAndUnknownValues,bufferSemanticsVisible:results.bufferSemanticsVisible,modalKeyboardFocusStaysInside:results.modalKeyboardFocusStaysInside,compactMetricsNoHorizontalOverflow:results.compactMetricsNoHorizontalOverflow,udpFormatAndQueueFields:results.udpFormatAndQueueFields,pausedSnapshotNotReportedAsPlaying:results.pausedSnapshotNotReportedAsPlaying,oldUdpExplainsMissingDiagnostics:results.oldUdpExplainsMissingDiagnostics,noSessionExplainsMissingSnapshot:results.noSessionExplainsMissingSnapshot,compactNoHorizontalOverflow:results.compactNoHorizontalOverflow,populatedModalGeometry,errors}));
  await browser.close();
  assert.deepEqual(errors,[]);
  for(const key of ['usbFormatAndUnknownValues','bufferSemanticsVisible','modalKeyboardFocusStaysInside','compactMetricsNoHorizontalOverflow','udpFormatAndQueueFields','pausedSnapshotNotReportedAsPlaying','oldUdpExplainsMissingDiagnostics','noSessionExplainsMissingSnapshot','compactNoHorizontalOverflow'])assert.equal(results[key],true,key);
  return;
 }
 if(process.env.ANPOD_WIRELESS_FLOW_ONLY){
  await page.setViewportSize({width:420,height:590});
  const dialog=page.getByRole('dialog',{name:'无线 Receiver'});
  const oneInterface=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}];
  const oneCandidate=[{endpoint:'192.168.50.3:45678'},{endpoint:'192.168.50.4:45678'}];
  await page.evaluate(({interfaces,candidates})=>{window.reviewWirelessFlowConfigure(interfaces,['phoneA-abcdef'],candidates);window.reviewSetUdpReady();},{interfaces:oneInterface,candidates:oneCandidate});
  results.listenerNotStartedAutomatically=await page.evaluate(()=>window.reviewListenerCalls.length===0);
  await page.getByRole('button',{name:'开始接收',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.length===1);
  results.singleClickStartsOnUniqueSelections=await page.evaluate(()=>window.reviewListenerCalls[0]?.localAddress==='192.168.50.2'&&window.reviewListenerCalls[0]?.bindingReference==='phoneA-abcdef'&&window.reviewListenerCalls[0]?.allowed===true);
  results.listenerStatusAndStopAction=await page.getByRole('button',{name:'停止接收',exact:true}).isEnabled()&&await page.evaluate(()=>window.reviewRead().wirelessConnectionOffers.automaticBinding==='phoneA-abcdef');
  await page.screenshot({path:`${out}/wireless-listening-auto.png`});
  await page.getByRole('button',{name:'停止接收',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.length===2);
  results.stopCancelsListener=await page.evaluate(()=>window.reviewListenerCalls[1]?.allowed===false&&!window.reviewRead().wirelessConnectionOffers.active);
  await page.evaluate(()=>window.reviewFailNextListener('模拟监听失败'));
  await page.getByRole('button',{name:'开始接收',exact:true}).click();
  await page.getByText(/模拟监听失败/).waitFor();
  results.listenerFailureHasRetry=await page.getByRole('button',{name:'开始接收',exact:true}).isEnabled()&&await page.evaluate(()=>!window.reviewRead().wirelessConnectionOffers.active);
  await page.getByRole('button',{name:'开始接收',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.length===4&&window.reviewRead().wirelessConnectionOffers.active);
  results.listenerRetryUsesCorrectBinding=await page.evaluate(()=>window.reviewListenerCalls[3]?.localAddress==='192.168.50.2'&&window.reviewListenerCalls[3]?.bindingReference==='phoneA-abcdef');
  await page.getByRole('button',{name:'停止接收',exact:true}).click();

  const multipleInterfaces=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]},{index:2,name:'Ethernet',address:'192.168.50.3',prefixLength:24,gateways:[]}];
  const multipleCandidates=[{endpoint:'192.168.50.5:45678'},{endpoint:'192.168.50.6:45678'}];
  await page.evaluate(({interfaces,candidates})=>{window.reviewWirelessFlowConfigure(interfaces,['phoneA-abcdef','phoneB-abcdef'],candidates);},{interfaces:multipleInterfaces,candidates:multipleCandidates});
  await page.getByRole('button',{name:'开始接收',exact:true}).click();
  await dialog.waitFor();
  await dialog.getByLabel('电脑网络接口').waitFor();
  await dialog.getByLabel('已配对接收端').waitFor();
  results.multipleChoicesRequireSelection=await dialog.getByLabel('电脑网络接口').inputValue()===''&&await dialog.getByLabel('已配对接收端').inputValue()==='';
  await dialog.getByLabel('电脑网络接口').selectOption('192.168.50.3');
  await dialog.getByLabel('已配对接收端').selectOption('phoneB-abcdef');
  await dialog.getByRole('button',{name:'刷新接口和配对信息',exact:true}).click();
  await page.waitForTimeout(100);
  results.validSelectionsSurviveRefresh=await dialog.getByLabel('电脑网络接口').inputValue()==='192.168.50.3'&&await dialog.getByLabel('已配对接收端').inputValue()==='phoneB-abcdef';

  const changedInterfaces=[{index:3,name:'Wi-Fi 2',address:'192.168.60.2',prefixLength:24,gateways:[]},{index:4,name:'Ethernet 2',address:'192.168.60.3',prefixLength:24,gateways:[]}];
  await page.evaluate(({interfaces,candidates})=>window.reviewWirelessFlowConfigure(interfaces,['phoneC-abcdef','phoneD-abcdef'],candidates),{interfaces:changedInterfaces,candidates:[{endpoint:'192.168.60.4:45678'},{endpoint:'192.168.60.5:45678'}]});
  await dialog.getByRole('button',{name:'刷新接口和配对信息',exact:true}).click();
  await page.waitForTimeout(100);
  results.invalidSelectionsAreRevoked=await dialog.getByLabel('电脑网络接口').inputValue()===''&&await dialog.getByLabel('已配对接收端').inputValue()==='';
  await dialog.getByLabel('电脑网络接口').selectOption('192.168.60.2');
  await dialog.getByLabel('已配对接收端').selectOption('phoneD-abcdef');
  await dialog.getByRole('button',{name:'刷新接口和配对信息',exact:true}).click();
  await page.waitForTimeout(100);
  results.userSelectionsSurviveRefresh=await dialog.getByLabel('电脑网络接口').inputValue()==='192.168.60.2'&&await dialog.getByLabel('已配对接收端').inputValue()==='phoneD-abcdef';
  await dialog.getByRole('button',{name:'开始监听',exact:true}).click();
  results.listenerUsesExplicitChoices=await page.evaluate(()=>window.reviewListenerCalls.at(-1)?.localAddress==='192.168.60.2'&&window.reviewListenerCalls.at(-1)?.bindingReference==='phoneD-abcdef');
  await dialog.getByText(/手机点击“开始接收”后自动认证/).waitFor();
  results.automaticModeDoesNotOfferPerRequestApproval=await dialog.getByRole('button',{name:'接受并连接所选手机',exact:true}).count()===0;
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'ambiguous-a',endpoint:'192.168.60.9:49772'},{id:'ambiguous-b',endpoint:'192.168.60.10:49772'}],true,'phoneD-abcdef'));
  results.multipleAutomaticTargetsRemainSelectable=await dialog.getByRole('button',{name:'接受并连接所选手机',exact:true}).isVisible();
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByRole('button',{name:'停止接收',exact:true}).click();

  await page.getByRole('button',{name:'选择目标，选择已配对接收端',exact:true}).click();
  const manualDialog=page.getByRole('dialog',{name:'无线 Receiver'});
  await manualDialog.waitFor();
  await manualDialog.getByLabel('已配对接收端').selectOption('phoneD-abcdef');
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'request-a',endpoint:'192.168.60.9:49772'}]));
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'request-a',endpoint:'192.168.60.9:49772'}],true,null));
  const requestA=manualDialog.getByRole('button',{name:/192\.168\.60\.9/});
  await requestA.waitFor();
  await page.waitForFunction(()=>document.querySelector('.offer-manager .candidate-list button[aria-pressed="true"]')?.textContent.includes('192.168.60.9'));
  const accept=manualDialog.getByRole('button',{name:'接受并连接所选手机',exact:true});
  results.singleRequestPrefilledButNotAccepted=await accept.isEnabled()&&await page.evaluate(()=>window.reviewAcceptedOffers.length===0);
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'request-a',endpoint:'192.168.60.9:49772'},{id:'request-b',endpoint:'192.168.60.10:49772'}],true,null));
  results.multiOfferPreservesStillValidChoice=await requestA.getAttribute('aria-pressed')==='true'&&await page.evaluate(()=>window.reviewAcceptedOffers.length===0);
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'request-b',endpoint:'192.168.60.10:49772'},{id:'request-c',endpoint:'192.168.60.11:49772'}],true,null));
  await page.waitForFunction(()=>![...document.querySelectorAll('.offer-manager .candidate-list button')].some(button=>button.getAttribute('aria-pressed')==='true'));
  results.expiredSelectionClearedWithoutAcceptance=await accept.isDisabled()&&await page.evaluate(()=>window.reviewAcceptedOffers.length===0);
  const requestB=dialog.getByRole('button',{name:/192\.168\.60\.10/});
  await requestB.click();
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'request-b',endpoint:'192.168.60.10:49772'},{id:'request-c',endpoint:'192.168.60.11:49772'}],true,null));
  results.explicitRequestSelectionSurvivesRefresh=await requestB.getAttribute('aria-pressed')==='true';
  await page.screenshot({path:`${out}/wireless-offer-accept.png`});
  await accept.click();
  results.onlyExplicitAcceptanceReachesBackend=await page.evaluate(()=>window.reviewAcceptedOffers.length===1&&window.reviewAcceptedOffers[0].offerId==='request-b'&&window.reviewAcceptedOffers[0].bindingReference==='phoneD-abcdef');
  results.activeSessionDoesNotStartListener=await page.getByRole('button',{name:'断开连接',exact:true}).isEnabled()&&await page.evaluate(()=>window.reviewListenerCalls.filter(call=>call.allowed).length===4);
  fs.writeFileSync(`${out}/wireless-flow-result.json`,JSON.stringify(results,null,2));
  const {layouts,...summary}=results;
  console.log(JSON.stringify(summary));
  await browser.close();
  assert.deepEqual(errors,[]);
  for(const key of ['listenerNotStartedAutomatically','singleClickStartsOnUniqueSelections','listenerStatusAndStopAction','stopCancelsListener','listenerFailureHasRetry','listenerRetryUsesCorrectBinding','multipleChoicesRequireSelection','validSelectionsSurviveRefresh','invalidSelectionsAreRevoked','userSelectionsSurviveRefresh','listenerUsesExplicitChoices','automaticModeDoesNotOfferPerRequestApproval','multipleAutomaticTargetsRemainSelectable','singleRequestPrefilledButNotAccepted','multiOfferPreservesStillValidChoice','expiredSelectionClearedWithoutAcceptance','explicitRequestSelectionSurvivesRefresh','onlyExplicitAcceptanceReachesBackend','activeSessionDoesNotStartListener'])assert.equal(results[key],true,key);
  return;
 }
 if(!process.env.ANPOD_SKIP_LAYOUT_MATRIX)for(const size of [[420,590],[360,500]]){
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
  if(process.env.ANPOD_USB_UPGRADE_ONLY){
   await page.setViewportSize({width:420,height:590});
   const openManager=async()=>{await page.getByRole('button',{name:'打开设置',exact:true}).click();await page.getByRole('button',{name:'检查或安装 Receiver',exact:true}).click();await page.getByRole('dialog',{name:'检查或安装 Receiver'}).waitFor();};
   const closeManager=async()=>page.getByRole('dialog',{name:'检查或安装 Receiver'}).getByRole('button',{name:'关闭',exact:true}).click();
   await page.evaluate(()=>window.reviewUsbScenario({active:true,activeVersion:6,binding:'phoneA-abcdef',candidate:{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:6,bindingReference:'phoneA-abcdef'},statuses:{TESTUSB:{installed:true,versionName:'0.6.0',versionCode:6,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();
   await page.getByRole('button',{name:'升级 Receiver · 9',exact:true}).waitFor();
   results.legacyWithoutNetworkUpdateBitStillOffersUsbUpgrade=await page.locator('.receiver-usb-card').innerText().then(text=>text.includes('当前版本码 6')&&text.includes('目标版本码 9'));
   await page.getByRole('button',{name:'升级 Receiver · 9',exact:true}).click();
   results.activePlaybackRequiresSingleConfirmation=await page.getByRole('dialog').count()===1&&await page.getByRole('dialog',{name:'停止播放并升级 Receiver？'}).count()===1;
   await page.getByRole('button',{name:'取消',exact:true}).click();
   await page.getByRole('dialog',{name:'检查或安装 Receiver'}).waitFor();
   await page.getByRole('button',{name:'升级 Receiver · 9',exact:true}).click();
   await page.getByRole('button',{name:'停止播放并继续',exact:true}).click();
   try { await page.getByText('USB 操作后已重新读取并核验 TESTUSB 的版本码 9。').waitFor({timeout:5000}); }
   catch(error) { console.log('USB upgrade failure page:', await page.locator('body').innerText()); await page.screenshot({path:`${out}/receiver-usb-upgrade-failure.png`}); throw error; }
   results.upgradeUsesSelectedBindingAndVerifiesActualStatus=await page.evaluate(()=>window.reviewCommandArgs.receiver_upgrade_usb?.serial==='TESTUSB'&&window.reviewCommandArgs.receiver_upgrade_usb?.bindingReference==='phoneA-abcdef'&&!window.reviewCalls.receiver_install&&window.reviewCalls.receiver_status>=2&&window.reviewCalls.refresh_devices===1);
   results.successHasNoOldNetworkUpdateState=await page.locator('.status-line').innerText().then(text=>!text.includes('Receiver 更新'))&&await page.getByText('USB 操作后已重新读取并核验 TESTUSB 的版本码 9。').count()===1;
   await page.screenshot({path:`${out}/receiver-usb-upgrade.png`});
   await closeManager();

   for(const version of [9,10]){
    await page.evaluate(v=>window.reviewUsbScenario({candidate:{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},statuses:{TESTUSB:{installed:true,versionName:'existing',versionCode:v,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}),version);
    await openManager();await page.getByText('当前版本已达到目标版本，无需升级。').waitFor();
    results[version===9?'sameVersionDoesNotOfferUpgrade':'newerVersionDoesNotOfferDowngrade']=await page.getByRole('button',{name:/升级 Receiver ·/}).count()===0;
    await closeManager();
   }
   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:11,versionName:'0.9.1-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},statuses:{TESTUSB:{installed:true,versionName:'unknown',versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();await page.getByText(/当前版本码 待 USB 核验/).waitFor();
   results.unknownVersionDoesNotAssumeUpdate=await page.getByRole('button',{name:/升级 Receiver ·/}).count()===0;
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:11,versionName:'0.9.1-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},devices:[{...{serial:'TESTUSB',transport:'usb',state:'unauthorized',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'未授权',connectable:false}}],statuses:{}}));
   await openManager();
   results.noOrUnauthorizedUsbIsNotSelectable=await page.getByLabel('已授权且在线的 USB 手机').locator('option').count()===1&&await page.getByText('连接 USB 手机并允许 USB 调试后，才能检查或升级。').count()===1;
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({candidate:null,statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();
   results.invalidTargetDoesNotOfferOperation=await page.getByText('本机 Receiver 安装包不可用').count()===1&&await page.getByRole('button',{name:/安装 Receiver ·|升级 Receiver ·/}).count()===0;
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:11,versionName:'0.9.1-usb',sizeBytes:5*1024*1024,currentVersionCode:8,bindingReference:'phoneA-abcdef'},bindings:['phoneA-abcdef','phoneB-abcdef'],active:true,binding:'phoneA-abcdef',activeVersion:8,devices:[{serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true},{serial:'TESTSECOND',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8',androidVersion:'14',apiLevel:'34',displayName:'Pixel 8',statusText:'可连接',connectable:true}],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'},TESTSECOND:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}},targetBindings:{TESTUSB:'phoneB-abcdef',TESTSECOND:'phoneB-abcdef'}}));
   await openManager();await page.getByRole('button',{name:/升级 Receiver · 11/}).waitFor();
   await page.getByLabel('已授权且在线的 USB 手机').selectOption('TESTSECOND');
   await page.getByRole('button',{name:'升级 Receiver · 11',exact:true}).click();
   await page.getByRole('button',{name:'停止播放并继续',exact:true}).click();
   await page.getByText(/USB 设备序列号或 Android 用户与所选 Receiver 绑定不匹配/).waitFor();
   results.wrongUsbTargetIsRejectedAndRetryRemains=await page.getByRole('button',{name:'升级 Receiver · 11',exact:true}).isVisible()&&await page.evaluate(()=>window.reviewGetUsbStatus('TESTSECOND').versionCode===8&&window.reviewCalls.receiver_upgrade_usb===1&&window.reviewRead().state==='streaming'&&window.reviewRead().sessionReceiverBinding==='phoneA-abcdef');
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:10,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:['unrelated-phone-abcdef'],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();await page.getByRole('button',{name:'USB 维护安装 · 10',exact:true}).click();
   await page.getByText('USB 操作后已重新读取并核验 TESTUSB 的版本码 10。').waitFor();
   results.unrelatedSavedBindingDoesNotBlockUsbMaintenance=await page.evaluate(()=>window.reviewCommandArgs.receiver_install?.serial==='TESTUSB'&&!window.reviewCalls.receiver_upgrade_usb&&window.reviewCalls.receiver_status>=2&&window.reviewCalls.refresh_devices===1);
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}},installError:'ADB 安装已取消'}));
   await openManager();await page.getByRole('button',{name:'安装 Receiver · 12',exact:true}).click();
   await page.getByText(/ADB 安装已取消/).waitFor();
   results.installFailureKeepsRetryAction=await page.getByRole('button',{name:'安装 Receiver · 12',exact:true}).isVisible()&&await page.evaluate(()=>window.reviewCalls.receiver_install===1&&window.reviewCalls.receiver_update_start===undefined);
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}}}));
   await openManager();await page.getByRole('button',{name:'安装 Receiver · 12',exact:true}).click();
   await page.getByText('USB 操作后已重新读取并核验 TESTUSB 的版本码 12。').waitFor();
   results.installReadsBackAndRefreshes=await page.evaluate(()=>window.reviewCommandArgs.receiver_install?.serial==='TESTUSB'&&window.reviewCalls.receiver_status>=2&&window.reviewCalls.refresh_devices===1);
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:['unrelated-phone-abcdef'],statuses:{TESTUSB:{installed:true,versionName:'0.6.0',versionCode:6,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await page.evaluate(()=>window.reviewUsbWirelessReady());
   await page.getByRole('button',{name:'选择目标，选择已配对接收端',exact:true}).click();
   const networkDialog=page.getByRole('dialog',{name:'无线 Receiver'});
   await networkDialog.waitFor();
   await networkDialog.locator('.network-advanced summary').click();
   const bootstrapMethod=networkDialog.getByRole('button',{name:'USB 首次绑定',exact:true});
   await bootstrapMethod.click();
   try { await page.waitForFunction(()=>{const dialog=document.querySelector('dialog[open][aria-label="无线 Receiver"]');return dialog?.querySelector('.network-connect')?.disabled===false;},{timeout:5000}); }
   catch(error){console.log('Old-compatible bootstrap dialog:',await networkDialog.innerText());console.log('Old-compatible mock calls:',await page.evaluate(()=>window.reviewCalls));throw error;}
   results.compatibleOldReceiverCanConnectWithOptionalUpgrade=await networkDialog.getByRole('button',{name:'USB 升级选项',exact:true}).isVisible()&&await networkDialog.getByRole('button',{name:'首次绑定并连接',exact:true}).isEnabled();
   await networkDialog.getByRole('button',{name:'首次绑定并连接',exact:true}).click();
   await page.waitForFunction(()=>window.reviewCalls.start_network_audio===1);
   results.oldReceiverStartsFirstBindWithoutUpgrade=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.bootstrapSerial==='TESTUSB'&&window.reviewRead().sessionTransport==='lan'&&window.reviewRead().sessionCapabilities?.versionCode===6);
   fs.writeFileSync(`${out}/result.json`,JSON.stringify(results,null,2));
   const {layouts,...summary}=results;
   console.log(JSON.stringify({...summary,layoutFailures:layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight)}));
   await browser.close();
   assert.deepEqual(errors,[]);
   if(!process.env.ANPOD_SKIP_LAYOUT_MATRIX)assert.equal(layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight||x.width!==x.size[0]).length,0);
   for(const key of ['legacyWithoutNetworkUpdateBitStillOffersUsbUpgrade','activePlaybackRequiresSingleConfirmation','upgradeUsesSelectedBindingAndVerifiesActualStatus','successHasNoOldNetworkUpdateState','sameVersionDoesNotOfferUpgrade','newerVersionDoesNotOfferDowngrade','unknownVersionDoesNotAssumeUpdate','noOrUnauthorizedUsbIsNotSelectable','invalidTargetDoesNotOfferOperation','wrongUsbTargetIsRejectedAndRetryRemains','unrelatedSavedBindingDoesNotBlockUsbMaintenance','installFailureKeepsRetryAction','installReadsBackAndRefreshes','compatibleOldReceiverCanConnectWithOptionalUpgrade','oldReceiverStartsFirstBindWithoutUpgrade'])assert.equal(results[key],true,key);
  return;
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
 await page.evaluate(()=>window.reviewCompletePair());await page.waitForTimeout(100);
 results.stalePairStaysClosed=await page.locator('dialog[open]').count()===0;
 await page.evaluate(()=>window.reviewSet('error','usb'));
 results.cleanupRetryEnabled=await page.getByRole('button',{name:'重试断开',exact:true}).isEnabled();
 await page.getByRole('button',{name:'重试断开',exact:true}).click();
 await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
 results.cleanupRetryWorks=await page.evaluate(()=>window.reviewCalls.disconnect===1);
 await page.locator('.device-select').click();
 await page.locator('.device-option').nth(1).click();
 results.selectDoesNotConnect=await page.evaluate(()=>window.reviewRead().selectedSerial==='TESTSECOND'&&!window.reviewRead().sessionSerial&&!(window.reviewCalls.connect_device_only||0));
 await page.getByRole('button',{name:'连接手机',exact:true}).click();
 await page.getByRole('button',{name:'断开连接',exact:true}).waitFor();
 results.connectKeepsComputerOutput=await page.evaluate(()=>window.reviewCalls.connect_device_only===1&&window.reviewRead().output==='computer');
 await page.locator('.output-card').filter({hasText:'手机'}).click();
 results.outputCardStartsPlayback=await page.evaluate(()=>window.reviewRead().output==='phone'&&window.reviewRead().state==='streaming');
 await page.locator('.device-select').click();await page.locator('.device-option').nth(0).click();
 results.switchNeedsConfirmation=await page.getByRole('dialog',{name:'切换连接手机？'}).count()===1;
 await page.getByRole('button',{name:'取消',exact:true}).click();
 results.cancelSwitchPreservesOutput=await page.evaluate(()=>window.reviewRead().sessionSerial==='TESTSECOND'&&window.reviewRead().output==='phone'&&!(window.reviewCalls.switch_device||0));
 await page.getByRole('button',{name:'低延迟',exact:true}).click();
 results.modeNeedsConfirmation=await page.locator('dialog[open]').count()===1;
 await page.getByRole('button',{name:'取消',exact:true}).click();
 results.cancelKeepsMode=await page.getByRole('button',{name:'均衡',exact:true}).getAttribute('aria-pressed')==='true';
 await page.evaluate(()=>window.reviewSet('connecting','wifi'));
 results.connectingCanBeCancelled=await page.getByRole('button',{name:'取消连接',exact:true}).isEnabled();
 results.busyBlocksModes=await page.getByRole('button',{name:'低延迟',exact:true}).isDisabled();
 await page.getByRole('button',{name:'取消连接',exact:true}).click();
 await page.waitForFunction(()=>window.reviewCalls.cancel_operation===1);
 results.cancelConnectionWorks=await page.getByRole('button',{name:'连接手机',exact:true}).isEnabled();
 await page.evaluate(()=>window.reviewSet('streaming','usb'));
 await page.getByRole('button',{name:'打开设置',exact:true}).click();
 await page.getByRole('checkbox',{name:/放大文字/}).check();
 await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
 await page.setViewportSize({width:360,height:500});await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
 results.largeTextNoOverflow=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('.device-card').getBoundingClientRect().height>=document.querySelector('.device-copy').getBoundingClientRect().height);
 await page.screenshot({path:`${out}/large-dark.png`});
 fs.writeFileSync(`${out}/result.json`,JSON.stringify(results,null,2));
 const {layouts,...summary}=results;
 console.log(JSON.stringify({...summary,layoutFailures:layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight)}));
 await browser.close();
 assert.deepEqual(errors,[]);
 if(!process.env.ANPOD_SKIP_LAYOUT_MATRIX)assert.equal(layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight||x.width!==x.size[0]).length,0);
 for(const key of ['focusStable','stalePairStaysClosed','cleanupRetryEnabled','cleanupRetryWorks','selectDoesNotConnect','connectKeepsComputerOutput','outputCardStartsPlayback','switchNeedsConfirmation','cancelSwitchPreservesOutput','modeNeedsConfirmation','cancelKeepsMode','connectingCanBeCancelled','busyBlocksModes','cancelConnectionWorks','largeTextNoOverflow'])assert.equal(results[key],true,key);
 assert.equal(results.codeRetained,'123456');
})().catch(e=>{console.error(e);process.exit(1)});
