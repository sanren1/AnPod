// Requires Playwright as an optional local check dependency; no test runner is used.
const path = require('path');
const { createRequire } = require('module');
const assert = require('assert/strict');
const frontendRequire = createRequire(path.join(__dirname, '../frontend/package.json'));
const { chromium } = process.env.ANPOD_PLAYWRIGHT_MODULE ? require(process.env.ANPOD_PLAYWRIGHT_MODULE) : frontendRequire('playwright');
const fs = require('fs');
const out = path.resolve(process.env.ANPOD_CHECK_OUTPUT || path.resolve(__dirname, '../.tmp-frontend-check'));
fs.mkdirSync(out, {recursive:true});
(async () => {
 const browser=await chromium.launch({executablePath:process.env.ANPOD_BROWSER_EXECUTABLE || undefined,headless:true});
 const page=await browser.newPage({viewport:{width:420,height:590},locale:'zh-CN'});
 page.setDefaultTimeout(10000);
 const errors=[];
 page.on('pageerror',e=>errors.push(e.message));
 await page.addInitScript(() => {
  if(location.search.includes('lang=en'))Object.defineProperty(navigator,'language',{configurable:true,get:()=> 'en-US'});
  let id=0;const callbacks=new Map(),listeners=new Map();window.reviewCopiedText='';Object.defineProperty(navigator,'clipboard',{configurable:true,value:{writeText:async value=>{window.reviewCopiedText=value;}}});
  const device={serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true};
  const secondDevice={...device,serial:'TESTSECOND',transport:'usb',displayName:'Long Google Pixel Wireless Device Name',androidVersion:'14'};
  let reviewInterfaces=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}];let reviewReceiverInterfaces=[{interfaceIndex:1,interfaceName:'Wi-Fi',address:'192.168.50.20',prefixLength:24}];let reviewBindings=['phoneA-abcdef','phoneB-abcdef'];let reviewCandidates=[{endpoint:'192.168.50.3:45678'}];let reviewOfferAddress=null;let reviewListenerError='';let reviewBackgroundStartError='';let reviewTrustError='';let reviewDeferTrust=false;let reviewTrustStatus={kind:'untilRevoked',persistence:'binding',remainingMs:null};
  let reviewReceiverManagementBusy=false;let reviewManagementSlotConflicts=0;
  let reviewAoaStartResolve=null;
  let reviewAoaTargets=[{id:'opaque-target-a',label:'AnPod 配件设备'}];let reviewAoaSwitchTargets=[{id:'switch-phone-a',label:'iQOO 11 (V2243A)'}];
  let snapshot={state:'noDevice',message:'尚未连接手机',deviceName:null,devices:[],selectedSerial:null,sessionSerial:null,sessionReceiverBinding:null,sessionGeneration:null,sessionTransport:null,sessionConnectionTransport:null,sessionWirelessProtocol:null,sessionCapabilities:null,sessionAndroidVersion:null,playbackDiagnostics:null,negotiatedAudioFormat:null,windowsMixFormat:null,androidOutputFormat:null,networkRecoverySecondsRemaining:null,wirelessConnectionOffers:{active:false,automaticBinding:null,interfaceAddress:null,interfacePrefixLength:null,offers:[]},transport:'usb',connectionTransport:'usb',output:'computer',audioBitrateBps:null,settings:{schemaVersion:1,autoConnectLastDevice:true,lastDeviceId:null,minimizeToTray:true,launchAtLogin:false,autoReconnectWifi:true,preferredTransport:'Usb',wirelessProtocol:'udp',audioBackend:'receiver',lastWirelessEndpoint:null,audioQuality:'standard',playbackMode:'Balanced',appearanceTheme:'System'},busy:false,canDisconnect:false,canChangeModes:true,statusDetail:'声音继续从电脑播放。',presence:[],version:'1.0.3'};
  window.reviewCalls={};window.reviewCommandArgs={};
  window.reviewSavedBindings=()=>[...reviewBindings];
  window.reviewSetAoaTargets=targets=>{reviewAoaTargets=structuredClone(targets);};
  window.reviewManagementSlotConflicts=()=>reviewManagementSlotConflicts;
  window.reviewManagementSlotBusy=()=>reviewReceiverManagementBusy;
  window.reviewChangeUsbDevices=devices=>{snapshot.devices=structuredClone(devices);emit();};
  window.reviewGetUsbStatus=serial=>structuredClone(reviewUsbStatuses[serial]);
  const emitEvent=(event,payload)=>{const listener=listeners.get(event);if(listener)callbacks.get(listener.handler)?.({event,id:listener.id,payload:structuredClone(payload)});};
  const emit=()=>emitEvent('anpod://snapshot',snapshot);
  window.reviewEmitReceiverProgress=payload=>emitEvent('anpod://receiver-install-progress',payload);
  window.reviewProgressListenerCount=()=>listeners.has('anpod://receiver-install-progress')?1:0;
  window.reviewRead=()=>structuredClone(snapshot);
  window.reviewReceiverVersionError=()=>{window.reviewUsbScenario();snapshot.state='error';snapshot.message='连接失败';snapshot.statusDetail='RECEIVER_UPDATE_REQUIRED:模拟版本核验的冗长底层诊断';snapshot.canDisconnect=false;emit();};
  let reviewCandidate={versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null};
  let reviewUsbStatuses={TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}};
  let reviewResetPreview={serial:'TESTUSB',androidUser:'0',bindingReference:'phoneA-abcdef',phoneState:'saved',phoneCommitment:'a'.repeat(64),pcState:'saved',pcCommitment:'b'.repeat(64),receiverBusy:false};let reviewResetResult=null;let reviewResetPreviewError='';let reviewResetError='';let reviewDeferReset=false;
  let reviewUpgradeError='';let reviewInstallError='';let reviewTargetBindings={TESTUSB:'phoneA-abcdef'};let reviewDeferReceiverStatus=false;let reviewDeferReceiverInterfaces=false;let reviewDeferInstall=false;let reviewDeferInstallVerification=false;let reviewManualInstallProgress=false;let reviewReceiverStatusError='';let reviewReceiverInterfacesError='';
  window.reviewUsbScenario=(options={})=>{
   window.reviewCalls={};window.reviewCommandArgs={};reviewReceiverManagementBusy=false;reviewManagementSlotConflicts=0;window.reviewManagementSlotTrace=[];
   reviewCandidate=options.candidate===undefined?{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null}:options.candidate;
   reviewUsbStatuses=options.statuses||{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}};
   reviewBindings=options.bindings===undefined?['phoneA-abcdef']:options.bindings;
   reviewResetPreview=options.resetPreview||{serial:'TESTUSB',androidUser:'0',bindingReference:'phoneA-abcdef',phoneState:'saved',phoneCommitment:'a'.repeat(64),pcState:'saved',pcCommitment:'b'.repeat(64),receiverBusy:false};reviewResetResult=options.resetResult||null;reviewResetPreviewError=options.resetPreviewError||'';reviewResetError=options.resetError||'';reviewDeferReset=false;
   reviewInterfaces=options.pcInterfaces||[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}];
   reviewReceiverInterfaces=options.receiverInterfaces||[{interfaceIndex:1,interfaceName:'Wi-Fi',address:'192.168.50.20',prefixLength:24}];
   reviewUpgradeError=options.upgradeError||'';reviewInstallError=options.installError||'';reviewTargetBindings=options.targetBindings||{TESTUSB:'phoneA-abcdef'};
   reviewReceiverStatusError=options.receiverStatusError||'';reviewReceiverInterfacesError=options.receiverInterfacesError||'';
   reviewDeferReceiverStatus=false;reviewDeferReceiverInterfaces=false;reviewDeferInstall=false;reviewDeferInstallVerification=false;reviewManualInstallProgress=false;
   snapshot={...snapshot,state:options.state|| (options.active?'streaming':'ready'),devices:options.devices||[device],selectedSerial:null,sessionSerial:options.active?(options.sessionSerial||null):null,sessionReceiverBinding:options.active?(options.binding||'phoneA-abcdef'):null,sessionTransport:options.active?(options.sessionSerial?'usb':'lan'):null,sessionConnectionTransport:options.active?(options.sessionSerial?'usb':'wireless'):null,sessionWirelessProtocol:options.active&&!options.sessionSerial?'udp':null,sessionCapabilities:options.active&&!options.sessionSerial?{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:0,versionCode:options.activeVersion||8}:null,canDisconnect:!!options.active,output:options.active?'phone':'computer',busy:false,settings:{...snapshot.settings,preferredTransport:options.active&&!options.sessionSerial?'Wifi':'Usb',wirelessProtocol:'udp'}};emit();
  };
  window.reviewHoldInstallAndVerification=()=>{reviewDeferInstall=true;reviewDeferInstallVerification=true;reviewManualInstallProgress=true;};
  window.reviewWirelessFlowConfigure=(interfaces,bindings,candidates)=>{reviewInterfaces=structuredClone(interfaces);reviewBindings=[...bindings];reviewCandidates=structuredClone(candidates);reviewOfferAddress=null;reviewListenerError='';snapshot.wirelessConnectionOffers={active:false,automaticBinding:null,interfaceAddress:null,interfacePrefixLength:null,offers:[]};emit();};
  window.reviewBackgroundStartFailure=message=>{reviewBackgroundStartError=message||'';};
  window.reviewBackgroundTrust=(status,error='')=>{reviewTrustStatus=structuredClone(status);reviewTrustError=error;};
  window.reviewDeferBackgroundTrust=()=>{reviewDeferTrust=true;};
  window.reviewTrustSessionGeneration=generation=>{snapshot.sessionGeneration=generation;emit();};
  window.reviewWirelessOffers=(offers,active=true,automaticBinding=snapshot.wirelessConnectionOffers.automaticBinding)=>{snapshot.wirelessConnectionOffers={active,automaticBinding,interfaceAddress:active?reviewOfferAddress:null,interfacePrefixLength:active?24:null,offers};emit();};
  window.reviewFailNextListener=(message)=>{reviewListenerError=message;};
  window.reviewDeferNextReceiverStatus=()=>{reviewDeferReceiverStatus=true;};
  window.reviewDeferNextReceiverInterfaces=()=>{reviewDeferReceiverInterfaces=true;};
  window.reviewDeferNextInstall=()=>{reviewDeferInstall=true;};
  window.reviewDeferNextReset=()=>{reviewDeferReset=true;};
  window.reviewClearReceiverErrors=()=>{reviewReceiverStatusError='';reviewReceiverInterfacesError='';};
  window.reviewClearInstallError=()=>{reviewInstallError='';};
  window.reviewAcceptedOffers=[];window.reviewListenerCalls=[];
  window.reviewDiagnosticsScenario=(scenario)=>{
   const udp=scenario==='udp'||scenario==='paused'||scenario==='oldUdp';
   const active=scenario!=='noSession';
   const values={sampleRate:48000,channels:2,receiverSampleRate:udp?44100:48000,receiverChannels:2,pcDroppedFrames:0,pcQueuedFrames:null,udpJitterTargetFrames:udp?480:null,udpExpiredFrames:udp?0:null,udpQueuedFrames:udp?96:null,tcpDroppedBacklogFrames:null,trackPendingFrames:192,underrunCount:scenario==='underrun'?4:0,underrunIncrement:scenario==='underrun'?2:null,playbackAdvancing:scenario==='paused'||scenario==='noProgress'?false:scenario==='missingProgress'||scenario==='oldUdp'||scenario==='noSession'?null:true,requestedBufferFrames:192,acceptedBufferFrames:240,effectiveBufferFrames:256,capacityFrames:768,startThresholdFrames:192,preRollFrames:96};
   const state=scenario==='noSession'?'noDevice':scenario==='paused'?'paused':scenario==='recovering'?'reconnecting':scenario==='cleanupError'?'error':scenario==='usb'||scenario==='udp'||scenario==='oldUdp'||scenario==='noProgress'||scenario==='missingProgress'||scenario==='underrun'?'streaming':'connected';
   snapshot={...snapshot,state,message:scenario==='cleanupError'?'模拟断开失败':active?'已连接 · 手机播放':'尚未连接',deviceName:active?'测试接收端':null,devices:[],selectedSerial:null,sessionSerial:active&&!udp?'TESTUSB':null,sessionReceiverBinding:active&&udp?'phoneA-abcdef':null,sessionTransport:active?(udp?'lan':'usb'):null,sessionConnectionTransport:active?(udp?'wireless':'usb'):null,sessionWirelessProtocol:udp?'udp':null,sessionCapabilities:udp?{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:scenario==='oldUdp'?4:12,versionCode:7}:null,playbackDiagnostics:scenario==='noSession'||scenario==='oldUdp'?null:values,connectionTransport:udp?'wireless':'usb',transport:udp?'wifi':'usb',output:scenario==='paused'||state==='streaming'?'phone':'computer',canDisconnect:active,busy:false,networkRecoverySecondsRemaining:scenario==='recovering'?17:null,statusDetail:scenario==='cleanupError'?'模拟断开错误，重试断开可继续清理。':'模拟诊断快照',settings:{...snapshot.settings,preferredTransport:udp?'Wifi':'Usb',wirelessProtocol:'udp'}};emit();
  };
  window.reviewDiagnosticsInjectPrivateMarkers=()=>{snapshot.deviceName='DEVICE_MARKER 192.168.77.23';snapshot.message='MESSAGE_MARKER FAKE_KEY_alpha SESSION_ID_beta';snapshot.statusDetail='DETAIL_MARKER 192.168.77.24 fake-key-gamma';snapshot.devices=[{serial:'SERIAL_MARKER 192.168.77.25'}];snapshot.sessionSerial='SERIAL_MARKER SESSION_ID_delta';snapshot.sessionReceiverBinding='REFERENCE_MARKER fake-key-epsilon';emit();};
  window.reviewSetUdpReady=()=>{window.reviewSet('ready','wifi');snapshot.sessionGeneration=null;snapshot.settings.wirelessProtocol='udp';snapshot.connectionTransport='wireless';snapshot.transport='wifi';emit();};
  window.reviewSetUdpStreaming=()=>{snapshot={...snapshot,state:'streaming',message:'手机正在播放',deviceName:'Google Pixel 8 Pro',sessionSerial:null,sessionReceiverBinding:'phoneA-abcdef',sessionTransport:'lan',sessionConnectionTransport:'wireless',sessionWirelessProtocol:'udp',sessionCapabilities:{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:12,versionCode:7},canDisconnect:true,busy:false,output:'phone',connectionTransport:'wireless',transport:'wifi',settings:{...snapshot.settings,preferredTransport:'Wifi',wirelessProtocol:'udp'}};emit();};
  window.reviewSetManagementOnly=()=>{snapshot={...snapshot,state:'managementOnly',message:'管理连接已认证，音频不兼容',deviceName:'Google Pixel 8 Pro',sessionSerial:null,sessionReceiverBinding:'phoneA-abcdef',sessionTransport:'lan',sessionConnectionTransport:'wireless',sessionWirelessProtocol:'udp',sessionCapabilities:{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:false,compatible:false,capabilityBits:4,versionCode:7},canDisconnect:true,busy:false,output:'computer',connectionTransport:'wireless',transport:'wifi',settings:{...snapshot.settings,preferredTransport:'Wifi',wirelessProtocol:'udp'}};emit();};
  window.reviewSetBusy=(busy=true)=>{snapshot.busy=busy;emit();};
  window.reviewSetAudioFormats=(negotiatedAudioFormat,windowsMixFormat,androidOutputFormat)=>{snapshot={...snapshot,negotiatedAudioFormat,windowsMixFormat,androidOutputFormat,output:'phone'};emit();};
  window.reviewActivateAoa=()=>{snapshot={...snapshot,state:'streaming',message:'USB Direct 已连接',deviceName:'AnPod 配件模式设备',sessionSerial:null,sessionReceiverBinding:'phoneA-abcdef',sessionTransport:'aoa',sessionConnectionTransport:'usbDirect',sessionWirelessProtocol:null,sessionCapabilities:null,canDisconnect:true,canChangeModes:false,busy:false,output:'phone',connectionTransport:'usb',settings:{...snapshot.settings,audioBackend:'receiver'}};emit();};
  window.reviewSetAudioQualityFailure=()=>{snapshot={...snapshot,state:'error',message:'Hi-Res 请求失败',statusDetail:'Hi-Res 需要 96 kHz Windows 混音源与 Receiver 支持；当前混音源为 48 kHz。请选择其他档位并重新连接。',canDisconnect:false};emit();};
  window.reviewRecoverySeconds=seconds=>{snapshot={...snapshot,state:'reconnecting',networkRecoverySecondsRemaining:seconds,busy:true,canDisconnect:true,sessionSerial:'TESTUSB',sessionTransport:'wifi',sessionConnectionTransport:'wireless',sessionWirelessProtocol:'adb',sessionAndroidVersion:'15',sessionCapabilities:null,output:'computer'};emit();};
  window.reviewSetActiveError=()=>{window.reviewSet('connected','usb');snapshot.state='error';snapshot.message='模拟播放失败详情';snapshot.statusDetail='电脑声音保持输出';emit();};
  window.reviewUsbWirelessReady=()=>{snapshot={...snapshot,state:'ready',devices:[device],transport:'wifi',connectionTransport:'wireless',output:'computer',settings:{...snapshot.settings,preferredTransport:'Wifi',wirelessProtocol:'udp'}};emit();};
  window.reviewSet=(state,transport='usb',serial='TESTUSB')=>{const all=state==='noDevice'?[]:[{...device,transport},{...secondDevice,transport}];const active=['connected','streaming','reconnecting'].includes(state);const current=all.find(d=>d.serial===serial);const protocol=transport==='wifi'?'adb':snapshot.settings.wirelessProtocol;snapshot={...snapshot,state,transport,connectionTransport:transport==='wifi'?'wireless':'usb',settings:{...snapshot.settings,preferredTransport:transport==='wifi'?'Wifi':'Usb',wirelessProtocol:protocol},devices:all,selectedSerial:active?serial:(snapshot.selectedSerial??serial),sessionSerial:active?serial:null,sessionReceiverBinding:null,sessionTransport:active?transport:null,sessionConnectionTransport:active?(transport==='wifi'?'wireless':'usb'):null,sessionWirelessProtocol:active?protocol:null,sessionCapabilities:null,sessionAndroidVersion:active?current?.androidVersion??null:null,deviceName:active?current?.displayName:state==='error'?'Google Pixel 8 Pro':null,canDisconnect:['connecting','connected','streaming','reconnecting','error'].includes(state),busy:['connecting','reconnecting'].includes(state),output:state==='streaming'?'phone':'computer'};emit();};
  window.reviewHideSessionDevice=()=>{snapshot.devices=snapshot.devices.filter(device=>device.serial!==snapshot.sessionSerial);emit();};
  window.reviewEmit=emit;
  window.__TAURI_EVENT_PLUGIN_INTERNALS__={unregisterListener() {}};
  window.__TAURI_INTERNALS__={transformCallback(fn){callbacks.set(++id,fn);return id;},unregisterCallback(cb){callbacks.delete(cb);},metadata:{currentWindow:{label:'main'},currentWebview:{label:'main'}},async invoke(cmd,args){
   window.reviewCalls[cmd]=(window.reviewCalls[cmd]||0)+1;window.reviewCommandArgs[cmd]=args;
   if(cmd==='plugin:event|listen'){const eventId=++id;listeners.set(args.event,{handler:args.handler,id:eventId});return eventId;}
   if(cmd==='plugin:event|unlisten'){if(listeners.get(args.event)?.id===args.eventId)listeners.delete(args.event);return;}
    if(cmd==='receiver_update_candidate')return structuredClone(reviewCandidate);
    if(cmd==='receiver_status'){
     if(reviewReceiverManagementBusy){reviewManagementSlotConflicts++;throw new Error('其他设备操作正在进行');}reviewReceiverManagementBusy=true;window.reviewManagementSlotTrace.push('receiver_status:start');
     try{if(reviewDeferReceiverStatus){reviewDeferReceiverStatus=false;await new Promise(resolve=>window.reviewResolveReceiverStatus=resolve);}if(reviewReceiverStatusError)throw new Error(reviewReceiverStatusError);const status=reviewUsbStatuses[args.serial];if(!status)throw new Error('没有找到已授权的 USB 设备');return structuredClone(status);}
     finally{reviewReceiverManagementBusy=false;window.reviewManagementSlotTrace.push('receiver_status:end');}
    }
   if(cmd==='receiver_network_interfaces'){
    if(reviewReceiverManagementBusy){reviewManagementSlotConflicts++;throw new Error('其他设备操作正在进行');}reviewReceiverManagementBusy=true;window.reviewManagementSlotTrace.push('receiver_network_interfaces:start');
    try{if(reviewDeferReceiverInterfaces){reviewDeferReceiverInterfaces=false;await new Promise(resolve=>window.reviewResolveReceiverInterfaces=resolve);}if(reviewReceiverInterfacesError)throw new Error(reviewReceiverInterfacesError);return structuredClone(reviewReceiverInterfaces);}
    finally{reviewReceiverManagementBusy=false;window.reviewManagementSlotTrace.push('receiver_network_interfaces:end');}
   }
   if(cmd==='receiver_binding_reset_preview'){
    if(reviewReceiverManagementBusy){reviewManagementSlotConflicts++;throw new Error('其他设备操作正在进行');}reviewReceiverManagementBusy=true;window.reviewManagementSlotTrace.push('receiver_binding_reset_preview:start');
    try{if(reviewResetPreviewError)throw new Error(reviewResetPreviewError);return structuredClone({...reviewResetPreview,serial:args.serial});}
    finally{reviewReceiverManagementBusy=false;window.reviewManagementSlotTrace.push('receiver_binding_reset_preview:end');}
   }
   if(cmd==='receiver_reset_binding'){
    if(reviewReceiverManagementBusy){reviewManagementSlotConflicts++;throw new Error('其他设备操作正在进行');}reviewReceiverManagementBusy=true;window.reviewManagementSlotTrace.push('receiver_reset_binding:start');
    try{if(reviewDeferReset){reviewDeferReset=false;await new Promise(resolve=>window.reviewResolveReset=resolve);}if(reviewResetError)throw new Error(reviewResetError);if(reviewResetResult)return structuredClone(reviewResetResult);reviewBindings=reviewBindings.filter(value=>value!==args.preview.bindingReference);return structuredClone({...args.preview,outcome:'reset',reason:'手机和 PC 记录均已清除。',nextAction:'如需配对，请单独启动 USB 首次绑定。'});}
    finally{reviewReceiverManagementBusy=false;window.reviewManagementSlotTrace.push('receiver_reset_binding:end');}
   }
    if(cmd==='receiver_install'||cmd==='receiver_upgrade_usb'){
     const event='anpod://receiver-install-progress';window.reviewInstallListenerBeforeInvoke=listeners.has(event);window.reviewLastReceiverOperationId=args.operationId;
     const progress=stage=>window.reviewEmitReceiverProgress({operationId:args.operationId,serial:args.serial,stage});
     progress('checking');if(reviewDeferInstall){reviewDeferInstall=false;await new Promise(resolve=>window.reviewResolveInstall=resolve);}
     if(!reviewManualInstallProgress)progress('transferring');
     if(cmd==='receiver_upgrade_usb'&&(reviewUpgradeError||reviewTargetBindings[args.serial]!==args.bindingReference)){progress('failed');if(reviewUpgradeError)throw new Error(reviewUpgradeError);throw new Error('USB 设备序列号或 Android 用户与所选 Receiver 绑定不匹配；未执行升级。');}
     if(cmd==='receiver_install'&&reviewInstallError){progress('failed');throw new Error(reviewInstallError);}
     reviewUsbStatuses[args.serial]={installed:true,versionName:reviewCandidate.versionName,versionCode:reviewCandidate.versionCode,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:cmd==='receiver_install'?'Receiver 已安装':'Receiver 已升级'};
     snapshot.state='ready';snapshot.sessionSerial=null;snapshot.sessionReceiverBinding=null;snapshot.canDisconnect=false;snapshot.output='computer';emit();
     if(!reviewManualInstallProgress)progress('verifying');
     if(reviewDeferInstallVerification){reviewDeferInstallVerification=false;await new Promise(resolve=>window.reviewResolveInstallVerification=resolve);}
     if(!reviewManualInstallProgress)progress('completed');
     return structuredClone(reviewUsbStatuses[args.serial]);
    }
    if(cmd==='refresh_devices')return structuredClone(snapshot);
   if(cmd==='begin_usb_direct_configuration')return {snapshot:structuredClone(snapshot),configurationId:1};
   if(cmd==='end_usb_direct_configuration')return structuredClone(snapshot);
   if(cmd==='receiver_binding_references')return [...reviewBindings];
   if(cmd==='list_aoa_targets')return structuredClone(reviewAoaTargets);
   if(cmd==='list_aoa_switch_targets')return structuredClone(reviewAoaSwitchTargets);
   if(cmd==='switch_aoa_mode'){window.reviewAoaSwitchArgs=args;return new Promise(resolve=>window.reviewCompleteAoaSwitch=result=>resolve(result));}
   if(cmd==='start_aoa_audio'){window.reviewAoaStartArgs=args;return new Promise(resolve=>{reviewAoaStartResolve=resolve;});}
   if(cmd==='network_interfaces')return structuredClone(reviewInterfaces);
   if(cmd==='start_network_audio'){if(reviewBackgroundStartError&&args.backgroundAuthorizationRequired){const message=reviewBackgroundStartError;reviewBackgroundStartError='';throw new Error(message);}if(args.backgroundAuthorizationRequired&&(!args.bindingReference||args.bootstrapSerial))throw new Error('后台授权目标缺少有效绑定');if(args.bootstrapSerial&&!reviewBindings.includes('new-phone-binding'))reviewBindings.push('new-phone-binding');snapshot={...snapshot,state:'connected',message:'Receiver 已认证',deviceName:'Google Pixel 8 Pro',sessionSerial:null,sessionReceiverBinding:args.bindingReference||'new-phone-binding',sessionGeneration:41,sessionTransport:'lan',sessionConnectionTransport:'wireless',sessionWirelessProtocol:'udp',sessionCapabilities:{controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:44,versionCode:reviewUsbStatuses[args.bootstrapSerial]?.versionCode??null},canDisconnect:true,connectionTransport:'wireless',transport:'wifi',output:'computer',settings:{...snapshot.settings,preferredTransport:'Wifi',wirelessProtocol:'udp'}};emit();return structuredClone(snapshot);}
   if(cmd==='query_background_trust'){if(reviewDeferTrust){reviewDeferTrust=false;await new Promise(resolve=>window.reviewResolveBackgroundTrust=resolve);}if(reviewTrustError)throw new Error(reviewTrustError);return {target:args.target.target,bindingReference:args.target.bindingReference,sessionGeneration:args.target.sessionGeneration??null,...structuredClone(reviewTrustStatus)};}
   if(cmd==='revoke_background_trust'){if(reviewTrustError)throw new Error(reviewTrustError);reviewTrustStatus={kind:'none',persistence:'none',remainingMs:0};if(args.target.target==='activeSession'){snapshot={...snapshot,state:'ready',sessionReceiverBinding:null,sessionGeneration:null,sessionTransport:null,sessionConnectionTransport:null,sessionWirelessProtocol:null,canDisconnect:false,output:'computer'};emit();}return {target:args.target.target,bindingReference:args.target.bindingReference,sessionGeneration:args.target.sessionGeneration??null,...structuredClone(reviewTrustStatus)};}
   if(cmd==='get_snapshot')return structuredClone(snapshot);
   if(cmd==='set_wireless_connection_offers'){window.reviewListenerCalls.push({localAddress:args.localAddress,allowed:args.allowed,bindingReference:args.bindingReference});if(args.allowed&&reviewListenerError){const message=reviewListenerError;reviewListenerError='';throw new Error(message);}if(args.allowed)reviewOfferAddress=args.localAddress;snapshot.wirelessConnectionOffers={active:args.allowed,automaticBinding:args.allowed?(args.bindingReference??(reviewBindings.length===1?reviewBindings[0]:null)):null,interfaceAddress:args.allowed?args.localAddress:null,interfacePrefixLength:args.allowed?24:null,offers:[]};emit();return structuredClone(snapshot.wirelessConnectionOffers);}
   if(cmd==='accept_wireless_connection_offer'){window.reviewAcceptedOffers.push({offerId:args.offerId,bindingReference:args.bindingReference});snapshot.wirelessConnectionOffers={active:false,automaticBinding:null,interfaceAddress:null,interfacePrefixLength:null,offers:[]};snapshot.state='connected';snapshot.message='已认证连接';snapshot.sessionSerial=null;snapshot.sessionReceiverBinding=args.bindingReference;snapshot.sessionTransport='lan';snapshot.sessionConnectionTransport='wireless';snapshot.sessionWirelessProtocol='udp';snapshot.sessionCapabilities={controlMajor:3,audioMajor:1,managementCompatible:true,audioCompatible:true,compatible:true,capabilityBits:12,versionCode:7};snapshot.canDisconnect=true;emit();return structuredClone(snapshot);}
   if(cmd==='discover_network_receivers')return structuredClone(reviewCandidates);
   if(cmd==='qr_credentials')return {instanceName:'studio-Test123456',password:'abcdefghijklmnopqrst',payload:'WIFI:T:ADB;S:studio-Test123456;P:abcdefghijklmnopqrst;;'};
   if(cmd==='pair_qr')return new Promise(resolve=>{window.reviewPairQrArgs=args;window.reviewCompleteQrPair=()=>resolve({success:true,message:'已配对',snapshot:structuredClone(snapshot)});});
   if(cmd==='pair_wifi')return new Promise(resolve=>{window.reviewCompletePair=()=>resolve({success:true,message:'已配对',snapshot:structuredClone(snapshot)});});
   if(cmd==='update_setting'){snapshot.settings[args.key]=args.value;if(args.key==='transport'){snapshot.connectionTransport=args.value;snapshot.transport=args.value==='wireless'?'wifi':'usb';snapshot.settings.preferredTransport=args.value==='wireless'?'Wifi':'Usb';}return structuredClone(snapshot);}
   if(cmd==='connect_device'){window.reviewSet('streaming',snapshot.transport);return {success:true,message:'已连接',snapshot:structuredClone(snapshot)};}
   if(cmd==='connect_device_only'){window.reviewSet('connected',snapshot.transport,args.serial);return {success:true,message:'已连接',snapshot:structuredClone(snapshot)};}
   if(cmd==='switch_device'){if(window.reviewSwitchMode==='failure')return {success:false,message:'模拟切换失败，已恢复原连接',snapshot:structuredClone(snapshot)};window.reviewSet(snapshot.output==='phone'?'streaming':'connected',snapshot.transport,args.serial);snapshot.selectedSerial=args.serial;return {success:true,message:'已切换',snapshot:structuredClone(snapshot)};}
   if(cmd==='select_device'){snapshot.selectedSerial=args.serial;return structuredClone(snapshot);}
   if(cmd==='cancel_operation'){const current={...snapshot,state:'ready',message:'已取消 USB Direct 连接',sessionTransport:null,sessionConnectionTransport:null,sessionWirelessProtocol:null,sessionReceiverBinding:null,sessionGeneration:null,canDisconnect:false,canChangeModes:true,busy:false,output:'computer'};snapshot=current;emit();if(reviewAoaStartResolve){const resolve=reviewAoaStartResolve;reviewAoaStartResolve=null;resolve(structuredClone(snapshot));}return structuredClone(snapshot);}
   if(cmd==='disconnect'){window.reviewSet('ready',snapshot.transport);return {success:true,message:'已断开',snapshot:structuredClone(snapshot)};}
   if(cmd==='set_output'){snapshot.output=args.output;snapshot.state=args.output==='phone'?'streaming':'connected';emit();return structuredClone(snapshot);}
   if(cmd==='reconfigure_session'){snapshot.settings[args.kind==='playback'?'playbackMode':'preferredTransport']=args.kind==='playback'?(args.value==='balanced'?'Balanced':'LowLatency'):(args.value==='usb'?'Usb':'Wifi');snapshot.transport=snapshot.settings.preferredTransport==='Wifi'?'wifi':'usb';window.reviewSet(snapshot.state,snapshot.transport);}
   return structuredClone(snapshot);
  }};
 });
 await page.goto(process.env.ANPOD_UI_URL || 'http://127.0.0.1:1420');
 await page.locator('.app-shell').waitFor();
 await page.waitForFunction(()=>window.reviewCalls?.get_snapshot===1&&window.reviewCalls?.refresh_devices===1);
 const results={errors,layouts:[],focusStable:null};
 if(process.env.ANPOD_AOA_ONLY){
  await page.setViewportSize({width:360,height:640});
  await page.getByRole('button',{name:'USB Direct（实验）',exact:true}).click();
  const dialog=page.getByRole('dialog',{name:'USB Direct（实验）'});
  await dialog.waitFor();
  const switchDetails=dialog.locator('details.aoa-switch-section');
  const switchSummary=switchDetails.locator('summary');
  await page.waitForFunction(()=>document.querySelector('.aoa-dialog details.aoa-switch-section')?.open===false);
  results.readyAoaPrioritizesConnection=!(await switchDetails.evaluate(el=>el.open))
    &&await dialog.getByLabel('USB Direct 设备').isVisible()
    &&await dialog.getByLabel('已配对接收端').isVisible()
    &&await dialog.getByRole('button',{name:'开始连接'}).isVisible();
  await switchSummary.focus();await switchSummary.press('Enter');
  results.keyboardDisclosureKeepsConsent=await switchDetails.evaluate(el=>el.open)
    &&await dialog.getByText(/允许暂时停止本机 ADB 服务（其管理的全部调试连接都会断开）；手机的 ADB\/MTP 也会暂时断开。/).isVisible();
  await switchSummary.press('Enter');
  await page.locator('.app-shell').evaluate(el=>el.classList.add('large-text'));
  results.readyAoaFits360LargeText=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('.aoa-dialog').scrollWidth<=document.querySelector('.aoa-dialog').clientWidth);
  await page.screenshot({path:`${out}/astra-ui-aoa-ready-360-large.png`});
  await switchSummary.click();await page.evaluate(()=>window.reviewEmit());
  results.manualDisclosureSurvivesRerender=await switchDetails.evaluate(el=>el.open)&&await switchSummary.evaluate(el=>document.activeElement===el);
  const switchTarget=dialog.getByLabel('已连接的手机');
  await switchTarget.focus();
  await switchTarget.selectOption('switch-phone-a');
  results.manualSelectionKeepsDisclosureOpen=await switchDetails.evaluate(el=>el.open)&&await switchTarget.evaluate(el=>document.activeElement===el);
  await switchSummary.click();
  const target=dialog.getByLabel('USB Direct 设备');
  const binding=dialog.getByLabel('已配对接收端');
  results.aoaNeedsExplicitChoices=await target.inputValue()===''&&await binding.inputValue()===''&&await dialog.getByRole('button',{name:'开始连接'}).isDisabled()&&await page.evaluate(()=>window.reviewCalls.start_aoa_audio===undefined);
  await target.selectOption('opaque-target-a');
  await binding.selectOption('phoneB-abcdef');
  await dialog.getByRole('button',{name:'开始连接'}).click();
  await page.waitForFunction(()=>window.reviewAoaStartArgs!==undefined);
  results.explicitTargetAndBindingReachBackend=await page.evaluate(()=>window.reviewAoaStartArgs?.targetId==='opaque-target-a'&&window.reviewAoaStartArgs?.bindingReference==='phoneB-abcdef'&&window.reviewCalls.start_aoa_audio===1);
  results.opaqueTargetIdIsNotDisplayed=!(await page.locator('body').innerText()).includes('opaque-target-a');
  await dialog.getByRole('button',{name:'取消连接'}).click();
  await page.waitForFunction(()=>window.reviewCalls.cancel_operation===1&&document.querySelector('dialog[open]')===null);
  results.cancelUsesExistingOperation=await page.evaluate(()=>window.reviewCalls.cancel_operation===1&&window.reviewRead().sessionTransport===null);
  await page.evaluate(()=>window.reviewSetAoaTargets([]));
  await page.getByRole('button',{name:'USB Direct（实验）',exact:true}).click();
  const firstSwitchDialog=page.getByRole('dialog',{name:'USB Direct（实验）'});
  const firstSwitch=firstSwitchDialog.locator('details.aoa-switch-section');
  await page.waitForFunction(()=>document.querySelector('.aoa-dialog details.aoa-switch-section')?.open===true);
  results.noTargetKeepsSwitchStepsDiscoverable=await firstSwitch.getByLabel('已连接的手机').isVisible();
  const previousAoaStarts=await page.evaluate(()=>window.reviewCalls.start_aoa_audio||0);
  await firstSwitch.getByLabel('已连接的手机').selectOption('switch-phone-a');
  await firstSwitch.getByRole('checkbox').check();
  const switchAction=firstSwitch.getByRole('button',{name:'切换 USB 模式',exact:true});
  await switchAction.focus();await switchAction.press('Enter');
  await page.waitForFunction(()=>window.reviewAoaSwitchArgs!==undefined);
  results.switchingKeepsStatusAndDetailsVisible=await firstSwitch.evaluate(el=>el.open)
    &&await firstSwitchDialog.locator('[role="status"]').filter({hasText:'正在切换 USB 模式…'}).isVisible();
  await page.evaluate(()=>{window.reviewSetAoaTargets([{id:'opaque-target-a',label:'AnPod 配件设备'}]);window.reviewCompleteAoaSwitch({success:true,message:'ok',requiresRefresh:true});});
  await page.waitForFunction(()=>document.querySelector('.aoa-dialog details.aoa-switch-section')?.open===false);
  results.successFocusIsVisible=await firstSwitchDialog.evaluate(dialog=>dialog.contains(document.activeElement)&&!!document.activeElement?.getClientRects().length);
  results.successCollapsesWithoutConnecting=await firstSwitchDialog.getByRole('button',{name:'开始连接'}).isDisabled()
    &&await page.evaluate(previous=>window.reviewCalls.start_aoa_audio===previous&&window.reviewCalls.switch_aoa_mode===1,previousAoaStarts);
  await firstSwitchDialog.getByRole('button',{name:'关闭'}).click();
  await page.evaluate(()=>window.reviewActivateAoa());
  results.actualAoaConnectionIsVisible=await page.locator('.device-select').innerText().then(text=>text.includes('USB Direct'));
  await page.getByRole('button',{name:'打开设置'}).click();
  const settings=page.getByRole('dialog');
  const backend=settings.getByLabel('手机接收方式');
  results.activeAoaLocksBackendAndMode=await backend.isDisabled()
    &&await settings.getByText(/USB Direct 会话使用 Receiver；请先断开 USB Direct，再更改接收后端。/).isVisible()
    &&!(await settings.innerText()).includes('UDP 使用 Receiver 后端')
    &&await page.getByRole('button',{name:'均衡'}).isDisabled()
    &&await page.getByRole('button',{name:'低延迟'}).isDisabled();
  await page.goto(`${(process.env.ANPOD_UI_URL || 'http://127.0.0.1:1420').replace(/\/$/,'')}?lang=en`);
  await page.locator('.app-shell').waitFor();
  await page.getByRole('button',{name:'USB Direct (experimental)',exact:true}).click();
  const englishDialog=page.getByRole('dialog',{name:'USB Direct (experimental)'});
  const englishSwitch=englishDialog.locator('details.aoa-switch-section');
  await page.waitForFunction(()=>document.querySelector('.aoa-dialog details.aoa-switch-section')?.open===false);
  await englishSwitch.locator('summary').press('Enter');
  results.englishDisclosureAndConsent=await englishDialog.getByLabel('USB Direct device').isVisible()
    &&await englishDialog.getByRole('button',{name:'Connect',exact:true}).isVisible()
    &&await englishDialog.getByText(/Allow briefly stopping this PC's ADB server \(all debugging connections it manages will disconnect\); this phone's ADB\/MTP will also briefly disconnect\./).isVisible();
  await englishSwitch.locator('summary').press('Enter');
  results.englishKeyboardCollapse=!(await englishSwitch.evaluate(el=>el.open));
  await page.locator('.app-shell').evaluate(el=>el.classList.add('large-text'));
  await page.setViewportSize({width:375,height:640});
  results.englishFits375LargeText=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('.aoa-dialog').scrollWidth<=document.querySelector('.aoa-dialog').clientWidth);
  fs.writeFileSync(`${out}/astra-ui-result.json`,JSON.stringify({
   readyAoaPrioritizesConnection:results.readyAoaPrioritizesConnection,
   keyboardDisclosureKeepsConsent:results.keyboardDisclosureKeepsConsent,
   readyAoaFits360LargeText:results.readyAoaFits360LargeText,
   manualDisclosureSurvivesRerender:results.manualDisclosureSurvivesRerender,
   aoaNeedsExplicitChoices:results.aoaNeedsExplicitChoices,
   noTargetKeepsSwitchStepsDiscoverable:results.noTargetKeepsSwitchStepsDiscoverable,
   switchingKeepsStatusAndDetailsVisible:results.switchingKeepsStatusAndDetailsVisible,
   successFocusIsVisible:results.successFocusIsVisible,
   successCollapsesWithoutConnecting:results.successCollapsesWithoutConnecting,
   explicitTargetAndBindingReachBackend:results.explicitTargetAndBindingReachBackend,
   cancelUsesExistingOperation:results.cancelUsesExistingOperation,
   englishDisclosureAndConsent:results.englishDisclosureAndConsent,
   englishKeyboardCollapse:results.englishKeyboardCollapse,
   englishFits375LargeText:results.englishFits375LargeText,
   errors
  },null,2));
  await englishDialog.locator('details.aoa-switch-section summary').focus();
  await page.screenshot({path:`${out}/astra-ui-aoa-english-375.png`});
  fs.writeFileSync(`${out}/aoa-result.json`,JSON.stringify(results,null,2));
  console.log(JSON.stringify({readyAoaPrioritizesConnection:results.readyAoaPrioritizesConnection,keyboardDisclosureKeepsConsent:results.keyboardDisclosureKeepsConsent,noTargetKeepsSwitchStepsDiscoverable:results.noTargetKeepsSwitchStepsDiscoverable,switchingKeepsStatusAndDetailsVisible:results.switchingKeepsStatusAndDetailsVisible,manualDisclosureSurvivesRerender:results.manualDisclosureSurvivesRerender,manualSelectionKeepsDisclosureOpen:results.manualSelectionKeepsDisclosureOpen,successFocusIsVisible:results.successFocusIsVisible,successCollapsesWithoutConnecting:results.successCollapsesWithoutConnecting,englishDisclosureAndConsent:results.englishDisclosureAndConsent,englishKeyboardCollapse:results.englishKeyboardCollapse,errors}));
  await browser.close();assert.deepEqual(errors,[]);
  for(const key of ['readyAoaPrioritizesConnection','keyboardDisclosureKeepsConsent','readyAoaFits360LargeText','manualDisclosureSurvivesRerender','manualSelectionKeepsDisclosureOpen','aoaNeedsExplicitChoices','noTargetKeepsSwitchStepsDiscoverable','switchingKeepsStatusAndDetailsVisible','successFocusIsVisible','successCollapsesWithoutConnecting','explicitTargetAndBindingReachBackend','opaqueTargetIdIsNotDisplayed','cancelUsesExistingOperation','englishDisclosureAndConsent','englishKeyboardCollapse','englishFits375LargeText'])assert.equal(results[key],true,key);
  return;
 }
 if(process.env.ANPOD_AUDIO_QUALITY_ONLY){
  await page.setViewportSize({width:360,height:640});
  await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
  await page.evaluate(()=>window.reviewSetAudioFormats(
   {codecVersion:2,sampleFormat:'pcm24le',sampleRateHz:48000,channels:2,validBitsPerSample:24,bytesPerSample:3,frameBytes:6},
   {sampleRate:48000,channels:2,sampleFormat:'float',containerBits:32,validBits:null},
   {sampleRate:48000,channels:2,encoding:'pcm24Packed'}));
  await page.getByRole('button',{name:/打开设置|settings/i}).click();
  let dialog=page.getByRole('dialog');
  await dialog.waitFor();
  const hifi=dialog.getByRole('radio',{name:/高保真/});
  const hires=dialog.getByRole('radio',{name:/Hi-Res/});
  results.zhLabelsAndDefault=await dialog.getByRole('radio',{name:'标准 · 48 kHz / 16-bit'}).isChecked()&&!await hifi.isDisabled()&&!await hires.isDisabled()&&await dialog.getByText(/标准音质仍是默认/).isVisible()&&await dialog.getByText(/仅在 Windows 混音源为 96 kHz 且 Receiver 支持时可用/).isVisible();
  results.zhActualFormats=await dialog.locator('.audio-quality-status').innerText().then(text=>text.includes('48 kHz · Packed PCM24LE · 2 ch · 24 有效位')&&text.includes('Float · 2 ch · 32 容器位 · 未知（尚未提供格式遥测）')&&text.includes('Packed PCM24 流'))&&await dialog.getByText(/手机物理输出设备的有效精度未知/).isVisible();
  await hifi.check();
  results.hifiCanBeRequested=await page.evaluate(()=>window.reviewRead().settings.audioQuality==='hifi');
  await page.evaluate(()=>window.reviewSetBusy(true));
  results.busyLocksQuality=await hifi.isDisabled()&&await hires.isDisabled();
  await page.evaluate(()=>window.reviewSetBusy(false));
  await hires.check();
  results.hiResCanBeRequested=await page.evaluate(()=>window.reviewRead().settings.audioQuality==='hires');
  results.selectedTierDoesNotInventActual96=await dialog.locator('.audio-quality-status').innerText().then(text=>text.includes('48 kHz · Packed PCM24LE · 2 ch · 24 有效位')&&!text.includes('96 kHz'));
  await page.evaluate(()=>window.reviewSetAudioFormats(
   {codecVersion:2,sampleFormat:'pcm24le',sampleRateHz:96000,channels:2,validBitsPerSample:24,bytesPerSample:3,frameBytes:6},
   {sampleRate:96000,channels:2,sampleFormat:'float',containerBits:32,validBits:24},
   {sampleRate:96000,channels:2,encoding:'pcm24Packed'}));
  results.zhHiResActualFormats=await dialog.locator('.audio-quality-status').innerText().then(text=>text.includes('96 kHz · Packed PCM24LE · 2 ch · 24 有效位')&&text.includes('96 kHz · Float · 2 ch · 32 容器位 · 24 有效位')&&text.includes('96 kHz · Packed PCM24 流'));
  await page.evaluate(()=>window.reviewSetAudioFormats(null,null,null));
  results.unknownFormatsStayUnknown=await dialog.locator('.audio-quality-status').innerText().then(text=>text.includes('未确认（需当前音频会话诊断）')&&text.includes('未知（尚未提供格式遥测）'));
  results.zhDialogNoHorizontalOverflow=await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth&&[...el.querySelectorAll('.audio-quality-status div')].every(row=>row.scrollWidth<=row.clientWidth));
  await page.keyboard.press('Tab');
  results.keyboardFocusStaysInSettings=await dialog.evaluate(el=>el.contains(document.activeElement)&&getComputedStyle(document.activeElement).outlineStyle!=='none');
  await dialog.evaluate(el=>el.scrollTop=0);
  await page.screenshot({path:`${out}/audio-quality-settings-zh-dark-360x640.png`});
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewSetAudioQualityFailure());
  const failure=page.getByRole('alert');
  await failure.locator('summary').click();
  results.unsupportedHiResFailureIsClear=await failure.innerText().then(text=>text.includes('Hi-Res 请求失败')&&text.includes('当前混音源为 48 kHz')&&text.includes('重新连接'));
  await page.addInitScript(()=>Object.defineProperty(navigator,'language',{configurable:true,get:()=> 'en-US'}));
  await page.reload();
  await page.locator('.app-shell').waitFor();
  await page.evaluate(()=>window.reviewSetAudioFormats(
   {codecVersion:1,sampleFormat:'pcm16le',sampleRateHz:48000,channels:2,validBitsPerSample:16,bytesPerSample:2,frameBytes:4},
   {sampleRate:44100,channels:2,sampleFormat:'integer',containerBits:24,validBits:null},
   {sampleRate:48000,channels:2,encoding:'float32'}));
  await page.getByRole('button',{name:/settings/i}).click();
  dialog=page.getByRole('dialog');await dialog.waitFor();
  results.enLabelsAndDefault=await dialog.getByRole('radio',{name:'Standard · 48 kHz / 16-bit'}).isChecked()&&!await dialog.getByRole('radio',{name:/Hi-Fi/}).isDisabled()&&!await dialog.getByRole('radio',{name:/Hi-Res/}).isDisabled()&&await dialog.getByText(/Standard remains the default/).isVisible()&&await dialog.getByText(/requires a 96 kHz Windows mix source and Receiver support/).isVisible();
  results.enActualFormats=await dialog.locator('.audio-quality-status').innerText().then(text=>text.includes('48 kHz · PCM16LE · 2 ch · 16 valid bits')&&text.includes('44.1 kHz · Integer PCM · 2 ch · 24 container bits · Unknown')&&text.includes('Float32 stream'))&&await dialog.getByText(/physical output device’s effective precision is unknown/).isVisible();
  results.enDialogGeometry=await dialog.evaluate(el=>({dialog:{scrollWidth:el.scrollWidth,clientWidth:el.clientWidth},quality:{scrollWidth:el.querySelector('.audio-quality-settings').scrollWidth,clientWidth:el.querySelector('.audio-quality-settings').clientWidth},rows:[...el.querySelectorAll('.audio-quality-status div')].map(row=>({scrollWidth:row.scrollWidth,clientWidth:row.clientWidth,text:row.innerText}))}));
  results.enDialogNoHorizontalOverflow=results.enDialogGeometry.dialog.scrollWidth<=results.enDialogGeometry.dialog.clientWidth&&results.enDialogGeometry.quality.scrollWidth<=results.enDialogGeometry.quality.clientWidth&&results.enDialogGeometry.rows.every(row=>row.scrollWidth<=row.clientWidth);
  await page.emulateMedia({colorScheme:'light',reducedMotion:'reduce'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
  await page.setViewportSize({width:375,height:640});
  await dialog.getByRole('checkbox',{name:/Larger text/i}).check();
  results.largeTextApplied=await page.locator('.app-shell.large-text').count()===1;
  results.englishLight375Geometry=await dialog.evaluate(el=>({dialog:{scrollWidth:el.scrollWidth,clientWidth:el.clientWidth},quality:{scrollWidth:el.querySelector('.audio-quality-settings').scrollWidth,clientWidth:el.querySelector('.audio-quality-settings').clientWidth},rows:[...el.querySelectorAll('.setting-toggle')].map(row=>({scrollWidth:row.scrollWidth,clientWidth:row.clientWidth,text:row.innerText}))}));
  results.englishLight375NoOverflow=results.englishLight375Geometry.dialog.scrollWidth<=results.englishLight375Geometry.dialog.clientWidth&&results.englishLight375Geometry.quality.scrollWidth<=results.englishLight375Geometry.quality.clientWidth;
  await dialog.evaluate(el=>el.scrollTop=0);
  await page.screenshot({path:`${out}/audio-quality-settings-en-light-large-375x640.png`});
  await page.setViewportSize({width:640,height:360});
  results.landscapeDialogCanScroll=await dialog.evaluate(el=>{const horizontal=el.scrollWidth<=el.clientWidth&&[...el.querySelectorAll('.audio-quality-status div')].every(row=>row.scrollWidth<=row.clientWidth);if(el.scrollHeight<=el.clientHeight)return horizontal;el.scrollTop=el.scrollHeight;return horizontal&&el.scrollTop>0;});
  fs.writeFileSync(`${out}/audio-quality-result.json`,JSON.stringify(results,null,2));
  console.log(JSON.stringify({zhLabelsAndDefault:results.zhLabelsAndDefault,zhActualFormats:results.zhActualFormats,hifiCanBeRequested:results.hifiCanBeRequested,hiResCanBeRequested:results.hiResCanBeRequested,selectedTierDoesNotInventActual96:results.selectedTierDoesNotInventActual96,zhHiResActualFormats:results.zhHiResActualFormats,unknownFormatsStayUnknown:results.unknownFormatsStayUnknown,unsupportedHiResFailureIsClear:results.unsupportedHiResFailureIsClear,busyLocksQuality:results.busyLocksQuality,zhDialogNoHorizontalOverflow:results.zhDialogNoHorizontalOverflow,keyboardFocusStaysInSettings:results.keyboardFocusStaysInSettings,enLabelsAndDefault:results.enLabelsAndDefault,enActualFormats:results.enActualFormats,enDialogNoHorizontalOverflow:results.enDialogNoHorizontalOverflow,largeTextApplied:results.largeTextApplied,englishLight375NoOverflow:results.englishLight375NoOverflow,englishLight375Geometry:results.englishLight375Geometry,landscapeDialogCanScroll:results.landscapeDialogCanScroll,errors}));
  await browser.close();assert.deepEqual(errors,[]);
  for(const key of ['zhLabelsAndDefault','zhActualFormats','hifiCanBeRequested','hiResCanBeRequested','selectedTierDoesNotInventActual96','zhHiResActualFormats','unknownFormatsStayUnknown','unsupportedHiResFailureIsClear','busyLocksQuality','zhDialogNoHorizontalOverflow','keyboardFocusStaysInSettings','enLabelsAndDefault','enActualFormats','enDialogNoHorizontalOverflow','largeTextApplied','englishLight375NoOverflow','landscapeDialogCanScroll'])assert.equal(results[key],true,key);
  return;
 }
 if(process.env.ANPOD_DIAGNOSTICS_ONLY){
  await page.setViewportSize({width:360,height:640});
  await page.emulateMedia({colorScheme:'light',reducedMotion:'reduce'});
  const openDiagnostics=async()=>page.getByRole('button',{name:/诊断/}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('usb'));
  await openDiagnostics();
  const dialog=page.getByRole('dialog',{name:'诊断'});
  results.technicalDetailsStartCollapsed=await dialog.locator('details').evaluate(el=>!el.open)&&await dialog.locator('details > summary').innerText()==='技术详情';
  results.summaryShowsStateConnectionAndMode=await dialog.locator('.diagnostic-overview').innerText().then(text=>text.includes('手机正在播放')&&text.includes('USB')&&text.includes('均衡')&&text.includes('最近一次 Receiver 快照确认播放推进'));
  await page.screenshot({path:`${out}/receiver-diagnostics-summary.png`});
  await dialog.locator('details > summary').click();
  results.usbFormatAndUnknownValues=await dialog.innerText().then(text=>text.includes('PC 采样率')&&text.includes('48,000 Hz')&&text.includes('Receiver 采样率')&&text.includes('PC 当前队列')&&text.includes('未提供')&&text.includes('PC 丢弃')&&text.includes('0 帧 · 会话累计'));
  results.bufferSemanticsVisible=await dialog.innerText().then(text=>['请求','系统接受','当前有效','容量','启动阈值','预填充','AudioTrack'].every(value=>text.includes(value)));
  await page.keyboard.press('Tab');
  results.modalKeyboardFocusStaysInside=await dialog.evaluate(el=>el.contains(document.activeElement));
  results.compactMetricsNoHorizontalOverflow=await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth&&[...el.querySelectorAll('.diagnostic-group dl')].every(dl=>dl.scrollWidth<=dl.clientWidth));
  const populatedModalGeometry=await dialog.evaluate(el=>({width:el.getBoundingClientRect().width,height:el.getBoundingClientRect().height,scrollHeight:el.scrollHeight,scrollWidth:el.scrollWidth}));
  await dialog.evaluate(el=>el.scrollTop=0);
  await page.screenshot({path:`${out}/receiver-diagnostics-compact.png`});
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>{window.reviewDiagnosticsScenario('udp');window.reviewDiagnosticsInjectPrivateMarkers();});
  await openDiagnostics();
  await dialog.locator('details > summary').click();
  const rawDetails=await dialog.locator('.diagnostic-details').innerText();
  results.rawDiagnosticDetailsRemainVisible=['MESSAGE_MARKER','DETAIL_MARKER','REFERENC'].every(marker=>rawDetails.includes(marker));
  await dialog.getByRole('button',{name:'复制诊断',exact:true}).click();
  const copiedText=await page.evaluate(()=>window.reviewCopiedText);
  results.copiedDiagnosticsUsesAllowlist=!['DEVICE_MARKER','MESSAGE_MARKER','DETAIL_MARKER','SERIAL_MARKER','REFERENCE_MARKER','REFERENC','192.168.77.23','192.168.77.24','192.168.77.25','FAKE_KEY_alpha','SESSION_ID_beta','SESSION_ID_delta','fake-key-gamma','fake-key-epsilon'].some(marker=>copiedText.includes(marker))
    &&copiedText.includes('44,100 Hz')&&copiedText.includes('PC 当前队列：未提供')&&copiedText.includes('PC 丢弃：0 帧 · 会话累计')&&copiedText.includes('480 帧 · 目标队列')&&copiedText.includes('协议兼容：管理 3（兼容 true），音频 1（兼容 true），位标志 0xc，版本码 7');
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('udp'));
  await openDiagnostics();
  await dialog.locator('details > summary').click();
  results.udpFormatAndQueueFields=await dialog.innerText().then(text=>text.includes('Receiver 采样率')&&text.includes('44,100 Hz')&&text.includes('UDP 抖动目标')&&text.includes('480 帧 · 目标队列')&&text.includes('UDP 过期')&&text.includes('0 帧 · 会话累计')&&text.includes('TCP 丢弃积压'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('paused'));
  await openDiagnostics();
  await dialog.locator('details > summary').click();
  results.pausedSnapshotNotReportedAsPlaying=await dialog.locator('.diagnostic-overview .diagnostic-context').innerText().then(text=>text.includes('手机播放已暂停'))&&await dialog.getByText('播放推进',{exact:true}).locator('..').innerText().then(text=>text.includes('否'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('oldUdp'));
  await openDiagnostics();
  results.oldUdpSummaryKeepsUnknownProgress=await dialog.locator('.diagnostic-overview .diagnostic-context').innerText().then(text=>text.includes('未提供播放推进数据'));
  await dialog.locator('details > summary').click();
  results.oldUdpExplainsMissingDiagnostics=await dialog.locator('.diagnostic-empty').innerText().then(text=>text.includes('未提供播放诊断能力')&&text.includes('可能为旧版本'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('missingProgress'));
  await openDiagnostics();
  results.missingProgressIsNotReportedAsPlaying=await dialog.locator('.diagnostic-overview').innerText().then(text=>text.includes('已连接，手机播放状态未确认')&&text.includes('目前没有播放推进数据，无法确认手机是否正在播放'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('recovering'));
  await openDiagnostics();
  results.recoveryUsesReportedRemainingSeconds=await dialog.locator('.diagnostic-overview').innerText().then(text=>text.includes('正在恢复连接（剩余 17 秒）'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('underrun'));
  await openDiagnostics();
  results.underrunIsNotPresentedAsAudibleFailure=await dialog.locator('.diagnostic-overview .diagnostic-context').innerText().then(text=>text.includes('欠载计数增加 2')&&text.includes('不等于可闻故障次数'));
  await dialog.locator('details > summary').click();
  results.underrunMetricKeepsUnknownAndCumulativeSemantics=await dialog.locator('.diagnostic-group').filter({hasText:'Receiver 播放状态'}).innerText().then(text=>text.includes('累计计数')&&text.includes('不等于可闻故障次数'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('cleanupError'));
  await openDiagnostics();
  results.cleanupFailureIsActionableAndRawErrorStaysCollapsed=await dialog.locator('.diagnostic-overview').innerText().then(text=>text.includes('断开未完成，点主按钮重试'))&&!await dialog.locator('.diagnostic-details').innerText().then(text=>text.includes('模拟断开错误'));
  results.cleanupFailureKeepsMainRetryAction=await page.locator('.main-action').innerText()==='重试断开';
  await dialog.locator('details > summary').click();
  results.cleanupFailureDetailsRetainOriginalMessage=await dialog.locator('.diagnostic-details').innerText().then(text=>text.includes('模拟断开错误'));
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.evaluate(()=>window.reviewDiagnosticsScenario('noSession'));
  await openDiagnostics();
  await dialog.locator('details > summary').click();
  results.noSessionExplainsMissingSnapshot=await dialog.locator('.diagnostic-empty').innerText().then(text=>text.includes('当前没有手机会话'));
  results.compactNoHorizontalOverflow=await dialog.evaluate(el=>el.scrollWidth<=el.clientWidth&&[...el.querySelectorAll('.diagnostic-group dl')].every(dl=>dl.scrollWidth<=dl.clientWidth));
  const modalGeometry=await dialog.evaluate(el=>({width:el.getBoundingClientRect().width,height:el.getBoundingClientRect().height,scrollHeight:el.scrollHeight,scrollWidth:el.scrollWidth}));
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.querySelector('dialog[open]')===null);
  results.diagnosticsEscapeClosesDialog=await page.locator('dialog[open]').count()===0;
  fs.writeFileSync(`${out}/diagnostics-result.json`,JSON.stringify({...results,populatedModalGeometry},null,2));
  console.log(JSON.stringify({technicalDetailsStartCollapsed:results.technicalDetailsStartCollapsed,summaryShowsStateConnectionAndMode:results.summaryShowsStateConnectionAndMode,usbFormatAndUnknownValues:results.usbFormatAndUnknownValues,bufferSemanticsVisible:results.bufferSemanticsVisible,modalKeyboardFocusStaysInside:results.modalKeyboardFocusStaysInside,compactMetricsNoHorizontalOverflow:results.compactMetricsNoHorizontalOverflow,rawDiagnosticDetailsRemainVisible:results.rawDiagnosticDetailsRemainVisible,copiedDiagnosticsUsesAllowlist:results.copiedDiagnosticsUsesAllowlist,udpFormatAndQueueFields:results.udpFormatAndQueueFields,pausedSnapshotNotReportedAsPlaying:results.pausedSnapshotNotReportedAsPlaying,oldUdpSummaryKeepsUnknownProgress:results.oldUdpSummaryKeepsUnknownProgress,oldUdpExplainsMissingDiagnostics:results.oldUdpExplainsMissingDiagnostics,missingProgressIsNotReportedAsPlaying:results.missingProgressIsNotReportedAsPlaying,recoveryUsesReportedRemainingSeconds:results.recoveryUsesReportedRemainingSeconds,underrunIsNotPresentedAsAudibleFailure:results.underrunIsNotPresentedAsAudibleFailure,underrunMetricKeepsUnknownAndCumulativeSemantics:results.underrunMetricKeepsUnknownAndCumulativeSemantics,cleanupFailureIsActionableAndRawErrorStaysCollapsed:results.cleanupFailureIsActionableAndRawErrorStaysCollapsed,cleanupFailureKeepsMainRetryAction:results.cleanupFailureKeepsMainRetryAction,cleanupFailureDetailsRetainOriginalMessage:results.cleanupFailureDetailsRetainOriginalMessage,noSessionExplainsMissingSnapshot:results.noSessionExplainsMissingSnapshot,compactNoHorizontalOverflow:results.compactNoHorizontalOverflow,diagnosticsEscapeClosesDialog:results.diagnosticsEscapeClosesDialog,populatedModalGeometry,errors}));
  await browser.close();
  assert.deepEqual(errors,[]);
  for(const key of ['technicalDetailsStartCollapsed','summaryShowsStateConnectionAndMode','usbFormatAndUnknownValues','bufferSemanticsVisible','modalKeyboardFocusStaysInside','compactMetricsNoHorizontalOverflow','rawDiagnosticDetailsRemainVisible','copiedDiagnosticsUsesAllowlist','udpFormatAndQueueFields','pausedSnapshotNotReportedAsPlaying','oldUdpSummaryKeepsUnknownProgress','oldUdpExplainsMissingDiagnostics','missingProgressIsNotReportedAsPlaying','recoveryUsesReportedRemainingSeconds','underrunIsNotPresentedAsAudibleFailure','underrunMetricKeepsUnknownAndCumulativeSemantics','cleanupFailureIsActionableAndRawErrorStaysCollapsed','cleanupFailureKeepsMainRetryAction','cleanupFailureDetailsRetainOriginalMessage','noSessionExplainsMissingSnapshot','compactNoHorizontalOverflow','diagnosticsEscapeClosesDialog'])assert.equal(results[key],true,key);
  return;
 }
 if(process.env.ANPOD_UI_SIMPLIFICATION_ONLY){
  await page.setViewportSize({width:420,height:590});
  const configure=async(bindings,candidates)=>{await page.evaluate(({bindings,candidates})=>{window.reviewWirelessFlowConfigure([{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}],bindings,candidates);window.reviewSetUdpReady();},{bindings,candidates});};
  const closeDialog=async dialog=>{await dialog.getByRole('button',{name:'关闭',exact:true}).click();await dialog.waitFor({state:'detached'});};
  const initialCandidates=[{endpoint:'192.168.50.3:45678'}];
  await configure(['phoneA-abcdef'],initialCandidates);
  await page.locator('.device-select').click();
  let dialog=page.getByRole('dialog',{name:'连接手机'});await dialog.waitFor();
  results.targetCardUsesSimpleConnectFlow=await dialog.locator('.connection-entry').isVisible()&&await dialog.getByRole('button',{name:'连接手机',exact:true}).isVisible()&&await dialog.getByLabel('电脑网络接口').count()===0;
  const targetEntryText=await dialog.locator('.connection-entry').innerText();await closeDialog(dialog);
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  dialog=page.getByRole('dialog',{name:'连接手机'});await dialog.waitFor();
  results.mainActionUsesSameConnectFlow=await dialog.locator('.connection-entry').innerText().then(text=>text===targetEntryText)&&await dialog.getByRole('button',{name:'连接手机',exact:true}).isVisible();
  await dialog.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===1);
  results.pairedPhoneUsesExistingAuthenticatedPath=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.bindingReference==='phoneA-abcdef'&&window.reviewCommandArgs.start_network_audio?.backgroundAuthorizationRequired===true&&window.reviewCalls.query_background_trust===undefined);
  await page.waitForFunction(()=>document.querySelector('dialog[open]')===null);
  await page.locator('.device-select').click();dialog=page.getByRole('dialog',{name:'连接手机'});await dialog.waitFor();
  results.activeTargetDoesNotDisconnect=await dialog.getByText('手机已连接。返回主界面可停止或切换播放。',{exact:true}).isVisible()&&await page.evaluate(()=>window.reviewCalls.disconnect===undefined);
  await closeDialog(dialog);

  await page.evaluate(()=>window.reviewUsbScenario({bindings:[],candidate:{versionCode:17,versionName:'0.17.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}}}));
  await configure([],[]);await page.evaluate(()=>window.reviewChangeUsbDevices([{serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true}]));await page.locator('.device-select').click();dialog=page.getByRole('dialog',{name:'连接手机'});
  results.firstUseExplainsUsbSetup=await dialog.getByText('首次使用需要先用 USB 连接手机并完成安全设置。点击继续后会引导你完成。',{exact:true}).isVisible()&&await dialog.getByRole('button',{name:'首次设置',exact:true}).isVisible();
  await dialog.getByRole('button',{name:'首次设置',exact:true}).click();
  const wizard=page.getByRole('dialog',{name:'首次设置'});await wizard.waitFor();
  await wizard.locator('.setup-wizard-step select').selectOption('TESTUSB');await wizard.getByRole('button',{name:'下一步',exact:true}).click();
  results.firstUseKeepsExplicitInstall=await wizard.getByText('Receiver 未安装',{exact:true}).count()===0&&await wizard.getByText('手机应用尚未安装',{exact:true}).isVisible()&&await wizard.getByRole('button',{name:'安装手机应用',exact:true}).isVisible();
  await closeDialog(wizard);

  await configure(['phoneA-abcdef','phoneB-abcdef'],initialCandidates);await page.locator('.device-select').click();dialog=page.getByRole('dialog',{name:'连接手机'});
  const targetPicker=dialog.getByLabel('选择已保存的手机');
  results.multipleSavedTargetsUseGenericLabels=await targetPicker.isVisible()&&await targetPicker.locator('option').allTextContents().then(items=>items.includes('已保存的手机 1')&&items.includes('已保存的手机 2')&&!items.some(item=>item.includes('abcdef')));
  await targetPicker.selectOption('phoneB-abcdef');await dialog.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===1);
  results.explicitSavedTargetIsUsed=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.bindingReference==='phoneB-abcdef');
  await page.waitForFunction(()=>document.querySelector('dialog[open]')===null);

  await configure(['phoneB-abcdef'],[{endpoint:'192.168.50.3:45678'},{endpoint:'192.168.50.4:45678'}]);await page.locator('.device-select').click();dialog=page.getByRole('dialog',{name:'连接手机'});
  await dialog.getByRole('button',{name:'连接手机',exact:true}).click();
  const recovery=page.getByRole('dialog',{name:'连接手机'});await recovery.getByLabel('手机地址').waitFor();
  results.discoveryAmbiguityMovesToRecovery=await recovery.getByRole('button',{name:'返回连接手机',exact:true}).isVisible()&&await recovery.getByRole('button',{name:'连接手机',exact:true}).isDisabled()&&await page.evaluate(()=>window.reviewCalls.start_network_audio===1);
  await page.evaluate(()=>window.reviewSetUdpStreaming());
  await recovery.getByRole('button',{name:/192\.168\.50\.4:45678/}).click();
  await recovery.getByRole('button',{name:'连接手机',exact:true}).click();
  const confirm=page.getByRole('dialog').filter({hasText:'断开当前会话并连接'});
  results.activeRecoveryRequiresConfirmation=await confirm.count()===1&&await page.evaluate(()=>window.reviewCalls.start_network_audio===1&&window.reviewCalls.disconnect===undefined);
  await confirm.getByRole('button',{name:'取消',exact:true}).click();await closeDialog(recovery);

  await configure(['phoneB-abcdef'],initialCandidates);await page.evaluate(()=>window.reviewBackgroundStartFailure('模拟连接失败'));
  await page.locator('.device-select').click();dialog=page.getByRole('dialog',{name:'连接手机'});await dialog.getByRole('button',{name:'连接手机',exact:true}).click();
  let recoveryDialog=page.getByRole('dialog',{name:'连接手机'});await recoveryDialog.getByRole('alert').waitFor();
  results.failedConnectShowsRecovery=await recoveryDialog.getByRole('button',{name:'返回连接手机',exact:true}).isVisible()&&await recoveryDialog.getByRole('button',{name:'连接手机',exact:true}).isEnabled();
  await recoveryDialog.getByRole('button',{name:'返回连接手机',exact:true}).click();dialog=page.getByRole('dialog',{name:'连接手机'});
  await dialog.getByRole('button',{name:'连接手机',exact:true}).click();await page.waitForFunction(()=>window.reviewCalls.start_network_audio===3);
  results.simpleRetryReconnects=await page.evaluate(()=>window.reviewRead().sessionConnectionTransport==='wireless');
  await page.waitForFunction(()=>document.querySelector('dialog[open]')===null);

  await page.evaluate(()=>window.reviewSet('connecting','wifi'));
  const cancel=page.getByRole('button',{name:'取消连接',exact:true});
  await cancel.evaluate(button=>{button.click();button.click();});
  await page.waitForFunction(()=>window.reviewCalls.cancel_operation===1);
  results.busyCancelSendsOneCommand=await page.evaluate(()=>window.reviewCalls.cancel_operation===1);

  await page.evaluate(()=>window.reviewSetUdpReady());
  await page.getByRole('button',{name:'打开设置',exact:true}).click();
  results.advancedConnectionAlwaysVisible=await page.getByRole('heading',{name:'高级连接',exact:true}).isVisible()&&await page.getByLabel('无线兼容方式').isVisible()&&await page.getByLabel('手机接收方式').isVisible()&&await page.getByRole('button',{name:'手机应用',exact:true}).isVisible();
  await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
  for(const [width,theme,largeText] of [[420,'light',false],[375,'light',false],[375,'dark',true]]){
   await page.setViewportSize({width,height:590});await page.emulateMedia({colorScheme:theme,reducedMotion:'reduce'});
   if(largeText){await page.getByRole('button',{name:'打开设置',exact:true}).click();await page.getByRole('checkbox',{name:/放大文字/}).check();await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();}
   await page.waitForTimeout(80);
   const geometry=await page.evaluate(()=>({width:document.documentElement.clientWidth,scrollWidth:document.documentElement.scrollWidth,deviceHeight:document.querySelector('.device-card').getBoundingClientRect().height,copyHeight:document.querySelector('.device-copy').getBoundingClientRect().height}));
   results.layouts??=[];results.layouts.push({width,theme,largeText,...geometry});
   if(width===420&&theme==='light')await page.screenshot({path:`${out}/connect-420-light.png`});
   if(width===375&&theme==='dark')await page.screenshot({path:`${out}/connect-375-dark-large.png`});
  }
  await page.evaluate(()=>window.reviewSetUdpReady());await page.locator('.device-select').focus();await page.keyboard.press('Enter');dialog=page.getByRole('dialog',{name:'连接手机'});await dialog.waitFor();await page.keyboard.press('Tab');
  results.keyboardFocusStaysInConnectDialog=await dialog.evaluate(el=>el.contains(document.activeElement));await page.keyboard.press('Escape');
  results.keyboardCanOpenAndCloseConnectFlow=results.keyboardFocusStaysInConnectDialog&&await page.locator('dialog[open]').count()===0;
  fs.writeFileSync(`${out}/result.json`,JSON.stringify(results,null,2));
  const {layouts,...summary}=results;console.log(JSON.stringify({...summary,layoutFailures:layouts.filter(x=>x.width!==x.scrollWidth||x.deviceHeight<x.copyHeight)}));
  await browser.close();assert.deepEqual(errors,[]);
  for(const key of ['targetCardUsesSimpleConnectFlow','mainActionUsesSameConnectFlow','pairedPhoneUsesExistingAuthenticatedPath','activeTargetDoesNotDisconnect','firstUseExplainsUsbSetup','firstUseKeepsExplicitInstall','multipleSavedTargetsUseGenericLabels','explicitSavedTargetIsUsed','discoveryAmbiguityMovesToRecovery','activeRecoveryRequiresConfirmation','failedConnectShowsRecovery','simpleRetryReconnects','busyCancelSendsOneCommand','advancedConnectionAlwaysVisible','keyboardFocusStaysInConnectDialog','keyboardCanOpenAndCloseConnectFlow'])assert.equal(results[key],true,key);
  assert.equal(layouts.filter(x=>x.width!==x.scrollWidth||x.deviceHeight<x.copyHeight).length,0);
  return;
 }
 if(process.env.ANPOD_RECEIVER_PROGRESS_ONLY){
  await page.setViewportSize({width:420,height:590});
  const openManager=async()=>{await page.getByRole('button',{name:'打开设置',exact:true}).click();await page.getByRole('button',{name:'手机应用',exact:true}).click();await page.getByRole('dialog',{name:'手机应用'}).waitFor();};
  const closeManager=async()=>{const dialog=page.getByRole('dialog',{name:'手机应用'});await dialog.getByRole('button',{name:'关闭',exact:true}).click();await dialog.waitFor({state:'detached'});};
  const uninstalled={TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}};
  await page.evaluate(({version,uninstalled})=>window.reviewUsbScenario({candidate:{versionCode:version,versionName:`0.${version}.0-usb`,sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:uninstalled}),{version:12,uninstalled});
  await openManager();
  let dialog=page.getByRole('dialog',{name:'手机应用'});
  let install=dialog.getByRole('button',{name:'安装手机应用',exact:true});
  await install.waitFor();await page.waitForFunction(()=>window.reviewCalls.receiver_status===1&&!document.querySelector('.receiver-status-actions button')?.disabled);
  await page.evaluate(()=>window.reviewHoldInstallAndVerification());
  await install.click();
  await dialog.getByText('检查手机',{exact:true}).waitFor();
  const operationId=await page.evaluate(()=>window.reviewCommandArgs.receiver_install?.operationId);
  results.progressListenerRegisteredBeforeInvoke=await page.evaluate(()=>!!window.reviewInstallListenerBeforeInvoke&&!!window.reviewCommandArgs.receiver_install?.operationId&&window.reviewProgressListenerCount()===1);
  results.checkingProgressIsNamedAndIndeterminate=await dialog.getByRole('progressbar',{name:'检查手机'}).count()===1&&await dialog.getByRole('button',{name:'安装手机应用',exact:true}).isDisabled();
  await page.screenshot({path:`${out}/receiver-install-checking.png`});
  await page.evaluate(id=>{
   window.reviewEmitReceiverProgress({operationId:`${id}-old`,serial:'TESTUSB',stage:'transferring'});
   window.reviewEmitReceiverProgress({operationId:id,serial:'TESTSECOND',stage:'verifying'});
  },operationId);
  await page.waitForTimeout(40);
  results.oldOperationAndOtherSerialProgressIgnored=await dialog.getByText('检查手机',{exact:true}).count()===1;
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'transferring'}),operationId);
  await dialog.getByRole('progressbar',{name:'安装手机应用'}).waitFor();
  results.transferringProgressIsNamed=await dialog.getByRole('progressbar',{name:'安装手机应用'}).count()===1;
  await page.screenshot({path:`${out}/receiver-install-transferring.png`});
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'verifying'}),operationId);
  await dialog.getByText('确认安装结果',{exact:true}).waitFor();
  results.verifyingProgressIsNamed=await dialog.getByRole('progressbar',{name:'确认安装结果'}).count()===1;
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'transferring'}),operationId);
  results.progressStageCannotRegress=await dialog.getByText('确认安装结果',{exact:true}).count()===1;
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'completed'}),operationId);
  results.commandReturnGatesCompletedStage=await dialog.getByText('确认安装结果',{exact:true}).count()===1;
  await page.evaluate(()=>window.reviewResolveInstall());
  await page.waitForFunction(()=>typeof window.reviewResolveInstallVerification==='function');
  await page.screenshot({path:`${out}/receiver-install-verifying.png`});
  await page.evaluate(()=>window.reviewResolveInstallVerification());
  await dialog.getByText('安装已完成',{exact:true}).waitFor();
  results.completionFollowsReturnedVersionCheck=await page.evaluate(()=>window.reviewCalls.receiver_install===1&&window.reviewCalls.receiver_status===1&&window.reviewCalls.refresh_devices===1&&window.reviewProgressListenerCount()===0);
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'failed'}),operationId);
  results.lateEventAfterCleanupIgnored=await dialog.getByText('安装已完成',{exact:true}).count()===1;
  await closeManager();

  await page.evaluate(({version,uninstalled})=>window.reviewUsbScenario({candidate:{versionCode:version,versionName:`0.${version}.0-usb`,sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:uninstalled,installError:'ADB 安装已取消'}),{version:13,uninstalled});
  await openManager();dialog=page.getByRole('dialog',{name:'手机应用'});install=dialog.getByRole('button',{name:'安装手机应用',exact:true});
  await install.waitFor();await install.click();
  await dialog.getByText('安装没有完成',{exact:true}).waitFor();
  const errorDetails=dialog.locator('.network-error details');
  await errorDetails.locator('summary').click();
  results.failedStageKeepsActualError=(await errorDetails.innerText()).includes('ADB 安装已取消')&&await page.evaluate(()=>window.reviewProgressListenerCount()===0);
  const failedOperationId=await page.evaluate(()=>window.reviewCommandArgs.receiver_install.operationId);
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'completed'}),failedOperationId);
  results.failureCannotBeOverwrittenByLateStage=await dialog.getByText('安装没有完成',{exact:true}).count()===1;
  await closeManager();

  await page.evaluate(({version,uninstalled})=>window.reviewUsbScenario({candidate:{versionCode:version,versionName:`0.${version}.0-usb`,sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:uninstalled}),{version:14,uninstalled});
  await openManager();dialog=page.getByRole('dialog',{name:'手机应用'});install=dialog.getByRole('button',{name:'安装手机应用',exact:true});
  await install.waitFor();await page.evaluate(()=>window.reviewHoldInstallAndVerification());await install.click();
  await dialog.getByText('检查手机',{exact:true}).waitFor();
  await page.evaluate(()=>{const button=[...document.querySelectorAll('dialog[open] button')].find(el=>el.textContent?.trim()==='安装手机应用');button?.click();button?.click();});
  results.busyBlocksRapidInstallClicks=await dialog.getByRole('button',{name:'安装手机应用',exact:true}).isDisabled()&&await page.evaluate(()=>window.reviewCalls.receiver_install===1);
  await page.evaluate(()=>window.reviewResolveInstall());await page.waitForFunction(()=>typeof window.reviewResolveInstallVerification==='function');await page.evaluate(()=>window.reviewResolveInstallVerification());
  await dialog.getByText('安装已完成',{exact:true}).waitFor();await closeManager();

  await page.evaluate(()=>window.reviewUsbScenario({active:true,sessionSerial:'TESTUSB',binding:'phoneA-abcdef',candidate:{versionCode:15,versionName:'0.15.0-usb',sizeBytes:5*1024*1024,currentVersionCode:8,bindingReference:'phoneA-abcdef'},bindings:['phoneA-abcdef'],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
  await openManager();dialog=page.getByRole('dialog',{name:'手机应用'});await dialog.getByRole('button',{name:'更新手机应用',exact:true}).waitFor();
  await dialog.locator('.receiver-maintenance-details > summary').click();
  await dialog.locator('.receiver-maintenance-details select').selectOption('phoneA-abcdef');
  await page.evaluate(()=>window.reviewHoldInstallAndVerification());await dialog.getByRole('button',{name:'更新手机应用',exact:true}).click();
  const confirm=page.getByRole('dialog',{name:'停止播放并更新手机应用？'});
  if(await confirm.count()===0){console.log('Active upgrade request state:',await page.locator('body').innerText());await page.screenshot({path:`${out}/receiver-upgrade-request.png`});throw new Error('Active upgrade confirmation did not open');}
  await confirm.getByRole('button',{name:'停止播放并继续',exact:true}).evaluate(button=>{button.click();button.click();});
  await page.waitForFunction(()=>window.reviewCalls.receiver_upgrade_usb===1&&typeof window.reviewResolveInstall==='function');
  results.confirmationCannotStartDuplicateCommands=await page.evaluate(()=>window.reviewCalls.receiver_upgrade_usb===1&&window.reviewProgressListenerCount()===1);
  const upgradeId=await page.evaluate(()=>window.reviewCommandArgs.receiver_upgrade_usb.operationId);
  await page.evaluate(id=>window.reviewEmitReceiverProgress({operationId:id,serial:'TESTUSB',stage:'verifying'}),upgradeId);
  await page.evaluate(()=>window.reviewResolveInstall());await page.waitForFunction(()=>typeof window.reviewResolveInstallVerification==='function');await page.evaluate(()=>window.reviewResolveInstallVerification());
  await page.getByText('安装已完成',{exact:true}).waitFor();
  results.upgradeReusesReturnedStatus=await page.evaluate(()=>window.reviewCalls.receiver_status===1&&window.reviewCalls.receiver_upgrade_usb===1&&window.reviewCalls.refresh_devices===1&&window.reviewProgressListenerCount()===0);
  await closeManager();

  await page.evaluate(({version,status})=>window.reviewUsbScenario({candidate:{versionCode:version,versionName:`0.${version}.0-usb`,sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],devices:[{serial:'TESTSECOND',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true}],statuses:{TESTSECOND:status}}),{version:16,status:uninstalled.TESTUSB});
  await page.locator('.device-select').click();
  await page.getByRole('dialog').getByRole('button',{name:'首次设置',exact:true}).click();
  const wizard=page.getByRole('dialog').filter({has:page.locator('.setup-wizard')});
  await wizard.locator('.setup-wizard-step select').selectOption('TESTSECOND');
  await wizard.getByRole('button',{name:'下一步',exact:true}).click();
  const wizardInstall=wizard.getByRole('button',{name:'安装手机应用',exact:true});
  if(await wizardInstall.count()===0)console.log('Setup wizard receiver state:',await wizard.innerText());
  await wizardInstall.waitFor();await page.evaluate(()=>window.reviewHoldInstallAndVerification());await wizardInstall.click();
  results.setupWizardShowsNamedProgressbar=await wizard.getByRole('progressbar',{name:'检查手机'}).count()===1;
  await page.evaluate(()=>window.reviewResolveInstall());await page.waitForFunction(()=>typeof window.reviewResolveInstallVerification==='function');await page.evaluate(()=>window.reviewResolveInstallVerification());
  await wizard.getByText('安装已完成',{exact:true}).waitFor();
  await wizard.getByRole('button',{name:'关闭',exact:true}).click();

  fs.writeFileSync(`${out}/receiver-progress-result.json`,JSON.stringify(results,null,2));
  console.log(JSON.stringify({...results,errors}));
  await browser.close();assert.deepEqual(errors,[]);
  for(const key of ['progressListenerRegisteredBeforeInvoke','checkingProgressIsNamedAndIndeterminate','oldOperationAndOtherSerialProgressIgnored','transferringProgressIsNamed','verifyingProgressIsNamed','progressStageCannotRegress','commandReturnGatesCompletedStage','completionFollowsReturnedVersionCheck','lateEventAfterCleanupIgnored','failedStageKeepsActualError','failureCannotBeOverwrittenByLateStage','busyBlocksRapidInstallClicks','confirmationCannotStartDuplicateCommands','upgradeReusesReturnedStatus','setupWizardShowsNamedProgressbar'])assert.equal(results[key],true,key);
  return;
 }
 if(process.env.ANPOD_SETUP_WIZARD_ONLY){
  await page.setViewportSize({width:375,height:640});
  const openWizard=async()=>{await page.locator('.device-select').click();await page.getByRole('dialog',{name:'选择手机'}).getByRole('button',{name:'首次设置',exact:true}).click();await page.getByRole('dialog',{name:'首次设置'}).waitFor();};
  const wizard=()=>page.getByRole('dialog',{name:'首次设置'});
  const usbDevice={serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true};
  const secondUsb={...usbDevice,serial:'TESTSECOND',displayName:'Pixel 8'};
  const status=(version=8)=>({installed:true,versionName:`0.${version}.0`,versionCode:version,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'});
  await page.evaluate(()=>window.reviewUsbScenario({state:'unauthorized',devices:[{serial:'TESTUSB',transport:'usb',state:'unauthorized',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'未授权',connectable:false}],statuses:{}}));
  await openWizard();
  results.wizardEntryVisible=await wizard().isVisible()&&await wizard().getByRole('list',{name:'设置步骤'}).count()===1;
  results.wizardFocusEntersModal=await wizard().evaluate(el=>el.contains(document.activeElement));
  results.wizardControlsMeet48pxTarget=await wizard().evaluate(el=>[...el.querySelectorAll('.setup-wizard button,.setup-wizard select')].every(control=>control.getBoundingClientRect().height>=48));
  results.unauthorizedUsbExplainsPermission=await wizard().getByText(/已检测到 USB 手机，但尚未授权/).isVisible()&&await wizard().getByRole('button',{name:'下一步',exact:true}).isDisabled();
  await page.screenshot({path:`${out}/setup-wizard-unauthorized.png`});
  await page.keyboard.press('Escape');
  results.cancelBeforeAuthorizationDoesNotInstallOrConnect=await page.locator('dialog[open]').count()===0&&await page.evaluate(()=>!window.reviewCalls.receiver_install&&!window.reviewCalls.start_network_audio);

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}},installError:'签名校验失败'}));
  await openWizard();
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByRole('button',{name:'安装手机应用',exact:true}).waitFor();
  results.uninstalledReceiverNeedsExplicitInstall=await page.evaluate(()=>!window.reviewCalls.receiver_install&&!window.reviewCalls.start_network_audio)&&await wizard().getByRole('button',{name:'下一步',exact:true}).isDisabled();
  await page.screenshot({path:`${out}/setup-wizard-install.png`});
  await wizard().getByRole('button',{name:'安装手机应用',exact:true}).click();
  await wizard().getByText('签名校验失败').waitFor();
  results.installSignatureFailureKeepsRetry=await wizard().getByRole('button',{name:'安装手机应用',exact:true}).isEnabled()&&await page.evaluate(()=>window.reviewCalls.receiver_install===1&&window.reviewCalls.start_network_audio===undefined);
  await page.evaluate(()=>window.reviewClearInstallError());
  await wizard().getByRole('button',{name:'安装手机应用',exact:true}).click();
  await wizard().getByText('已重新检查并确认安装结果码 12。').waitFor();
  results.installRetryUsesReturnedVerifiedStatus=await page.evaluate(()=>window.reviewCalls.receiver_install===2&&window.reviewCalls.receiver_status===1&&window.reviewCalls.refresh_devices===1);
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByLabel('电脑网络接口').waitFor();
  results.wizardCanContinueAfterVerifiedInstall=await wizard().getByLabel('电脑网络接口').count()===1&&await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).isEnabled();
  await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===1);
  results.firstBindUsesBootstrapAndLeavesOutputOnComputer=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.bootstrapSerial==='TESTUSB'&&window.reviewCommandArgs.start_network_audio?.receiverAddress==='192.168.50.20'&&window.reviewRead().sessionTransport==='lan'&&window.reviewRead().output==='computer'&&!window.reviewCalls.set_output);
   await page.locator('.device-select').click();
   const addAnotherDialog=page.getByRole('dialog',{name:'连接手机'});await addAnotherDialog.waitFor();
   results.savedBindingKeepsUsbSetupPath=await addAnotherDialog.getByRole('button',{name:'首次设置（USB）',exact:true}).isVisible()&&await addAnotherDialog.getByRole('button',{name:'添加手机',exact:true}).count()===0&&await addAnotherDialog.getByLabel('已配对手机').inputValue()==='new-phone-binding';
  await addAnotherDialog.getByRole('button',{name:'关闭',exact:true}).click();

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{...{installed:true,versionName:'0.6.0',versionCode:6,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}}));
  await openWizard();
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByText('当前版本兼容时可直接继续；升级是可选操作。').waitFor();
  results.compatibleOldReceiverCanSkipUpgrade=await wizard().getByRole('button',{name:/可选维护升级/}).isVisible()&&await wizard().getByRole('button',{name:'下一步',exact:true}).isEnabled();
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await page.keyboard.press('Escape');
  results.cancelBeforeBindingDoesNotConnect=await page.locator('dialog[open]').count()===0&&await page.evaluate(()=>window.reviewCalls.start_network_audio===undefined);

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:['unrelated-phone-abcdef'],statuses:{TESTUSB:{installed:true,versionName:'0.6.0',versionCode:6,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByRole('button',{name:/可选维护升级 · 12/}).click();
  await wizard().getByText('已重新检查并确认安装结果码 12。').waitFor();
  results.unrelatedSavedBindingUsesIndependentUsbMaintenance=await page.evaluate(()=>window.reviewCommandArgs.receiver_install?.serial==='TESTUSB'&&!Object.hasOwn(window.reviewCommandArgs.receiver_install||{},'bindingReference')&&!window.reviewCalls.receiver_upgrade_usb);
  await page.keyboard.press('Escape');

  const localChoices=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]},{index:2,name:'Ethernet',address:'10.0.0.2',prefixLength:24,gateways:[]}];
  const phoneChoices=[{interfaceIndex:1,interfaceName:'Wi-Fi',address:'192.168.50.20',prefixLength:24},{interfaceIndex:2,interfaceName:'Ethernet',address:'10.0.0.20',prefixLength:24}];
  await page.evaluate(({locals,phones})=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:['phoneA-abcdef'],devices:[{serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true},{serial:'TESTSECOND',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8',androidVersion:'14',apiLevel:'34',displayName:'Pixel 8',statusText:'可连接',connectable:true}],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'},TESTSECOND:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}},pcInterfaces:locals,receiverInterfaces:phones}),{locals:localChoices,phones:phoneChoices});
  await openWizard();
  results.multipleUsbDevicesRequireChoice=await wizard().getByLabel('已授权且在线的 USB 手机').inputValue()===''&&await wizard().getByRole('button',{name:'下一步',exact:true}).isDisabled();
  await wizard().getByLabel('已授权且在线的 USB 手机').selectOption('TESTSECOND');
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByText(/Receiver 已安装/).waitFor();
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByLabel('电脑网络接口').selectOption('10.0.0.2');
  await wizard().getByLabel('手机当前 IPv4 地址').selectOption('10.0.0.20');
  results.multipleNetworkInterfacesRequireExplicitMatching=await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).isEnabled();
  await page.screenshot({path:`${out}/setup-wizard-disambiguation.png`});
  await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===1);
  results.disambiguatedBootstrapUsesChosenPhoneAndInterfaces=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.bootstrapSerial==='TESTSECOND'&&window.reviewCommandArgs.start_network_audio?.localAddress==='10.0.0.2'&&window.reviewCommandArgs.start_network_audio?.receiverAddress==='10.0.0.20');

  await page.evaluate(({locals,phones})=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}},pcInterfaces:locals,receiverInterfaces:phones}),{locals:[localChoices[0]],phones:[{interfaceIndex:3,interfaceName:'Mobile',address:'172.16.0.20',prefixLength:24}]});
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  results.noSameSubnetHasActionAndBlocksBinding=await wizard().getByText(/没有同网段接口/).isVisible()&&await wizard().getByRole('button',{name:'重新检查网络接口',exact:true}).isEnabled()&&await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).isDisabled();
  await page.keyboard.press('Escape');

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByText(/Receiver 已安装/).waitFor();
  await page.evaluate(()=>window.reviewDeferNextReceiverStatus());
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await page.waitForFunction(()=>typeof window.reviewResolveReceiverStatus==='function');
  await page.keyboard.press('Escape');
  await page.evaluate(()=>window.reviewResolveReceiverStatus());
  await page.waitForTimeout(80);
  results.lateReceiverStatusAfterNextDoesNotReopenWizard=await page.locator('dialog[open]').count()===0&&await page.evaluate(()=>window.reviewCalls.start_network_audio===undefined);

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],receiverStatusError:'Receiver 状态读取失败',statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByText('Receiver 状态读取失败').waitFor();
  results.receiverStatusFailureStaysOnStepTwo=await wizard().locator('.setup-step[aria-current="step"] span').innerText()==='2'&&await wizard().getByRole('button',{name:'下一步',exact:true}).isDisabled();
  await page.evaluate(()=>window.reviewClearReceiverErrors());
  await wizard().getByRole('button',{name:'重新检查',exact:true}).click();
  await page.waitForFunction(()=>{const dialog=document.querySelector('dialog[open]');return dialog?.querySelector('.setup-step[aria-current="step"] span')?.textContent==='2'&&!dialog.querySelector('.setup-wizard-actions .primary-button')?.disabled;});
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByLabel('电脑网络接口').waitFor();
  results.receiverStatusRetryAdvancesOnlyAfterSuccess=await wizard().locator('.setup-step[aria-current="step"] span').innerText()==='3';
  await page.keyboard.press('Escape');

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],receiverInterfacesError:'手机网络接口读取失败',statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByText('手机网络接口读取失败').waitFor();
  results.receiverInterfacesFailureStaysOnStepTwo=await wizard().locator('.setup-step[aria-current="step"] span').innerText()==='2'&&await wizard().getByRole('button',{name:'下一步',exact:true}).isDisabled();
  await page.evaluate(()=>window.reviewClearReceiverErrors());
  await wizard().getByRole('button',{name:'重新检查',exact:true}).click();
  await page.waitForFunction(()=>!document.querySelector('dialog[open] .receiver-status-actions button')?.disabled);
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByLabel('电脑网络接口').waitFor();
  results.receiverInterfacesRetryAdvancesOnlyAfterSuccess=await wizard().locator('.setup-step[aria-current="step"] span').innerText()==='3';
  await page.keyboard.press('Escape');
  await page.waitForFunction(()=>document.querySelector('dialog[open]')===null);

  await page.evaluate(({second})=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],devices:[{serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true},second],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'},TESTSECOND:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}),{second:secondUsb});
  await openWizard();await wizard().getByLabel('已授权且在线的 USB 手机').selectOption('TESTUSB');
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();await wizard().getByText(/Receiver 已安装/).waitFor();
  await page.evaluate(()=>window.reviewDeferNextReceiverInterfaces());
  await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await page.waitForFunction(()=>typeof window.reviewResolveReceiverInterfaces==='function');
  await page.evaluate(({second})=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],devices:[second],statuses:{TESTSECOND:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}),{second:secondUsb});
  await wizard().getByRole('button',{name:'上一步',exact:true}).click();
  await wizard().getByLabel('已授权且在线的 USB 手机').selectOption('TESTSECOND');
  await page.waitForFunction(()=>window.reviewCommandArgs.receiver_status?.serial==='TESTSECOND');
  await page.evaluate(()=>window.reviewResolveReceiverInterfaces());
  results.deviceChangeInvalidatesPendingInterfaceCheck=await wizard().locator('.setup-step[aria-current="step"] span').innerText()==='1'&&await page.evaluate(()=>window.reviewCalls.start_network_audio===undefined);
  await page.keyboard.press('Escape');

  await page.evaluate(()=>{window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}}});window.reviewDeferNextInstall();});
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();await wizard().getByRole('button',{name:'安装手机应用',exact:true}).click();
  await page.waitForFunction(()=>typeof window.reviewResolveInstall==='function');
  await page.keyboard.press('Escape');
  await page.evaluate(()=>window.reviewResolveInstall());
  await page.getByText(/Receiver 版本码 12 已核验/).waitFor();
  results.closedWizardDoesNotCancelOrReopenCommittedInstall=await page.locator('dialog[open]').count()===0&&await page.evaluate(()=>window.reviewCalls.receiver_install===1&&window.reviewCalls.refresh_devices===1&&!window.reviewCalls.start_network_audio);

  await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
  await page.getByRole('button',{name:'打开设置',exact:true}).click();
  await page.getByRole('checkbox',{name:/放大文字/}).check();
  await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
  await page.setViewportSize({width:360,height:430});await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});
  await openWizard();await wizard().getByRole('button',{name:'下一步',exact:true}).click();await wizard().getByRole('button',{name:'下一步',exact:true}).click();
  await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).waitFor();
  results.largeTextWizardNoHorizontalOverflow=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('dialog[open]').scrollWidth<=document.querySelector('dialog[open]').clientWidth);
  await wizard().evaluate(el=>el.scrollTop=0);
  await page.setViewportSize({width:360,height:720});
  await page.screenshot({path:`${out}/setup-wizard-large-dark.png`});
  await page.setViewportSize({width:360,height:430});
  await wizard().getByRole('button',{name:'建立绑定并连接',exact:true}).scrollIntoViewIfNeeded();
  results.largeTextWizardCanScrollToPrimaryAction=await wizard().evaluate(el=>el.scrollTop>0&&el.querySelector('.setup-wizard').scrollWidth<=el.clientWidth);

  fs.writeFileSync(`${out}/setup-wizard-result.json`,JSON.stringify(results,null,2));
  console.log(JSON.stringify(results));
  await browser.close();
  assert.deepEqual(errors,[]);
  for(const key of ['wizardEntryVisible','wizardFocusEntersModal','wizardControlsMeet48pxTarget','unauthorizedUsbExplainsPermission','cancelBeforeAuthorizationDoesNotInstallOrConnect','uninstalledReceiverNeedsExplicitInstall','installSignatureFailureKeepsRetry','installRetryUsesReturnedVerifiedStatus','wizardCanContinueAfterVerifiedInstall','firstBindUsesBootstrapAndLeavesOutputOnComputer','savedBindingOffersAddPhoneEntry','addPhoneReusesUniqueSavedBinding','compatibleOldReceiverCanSkipUpgrade','cancelBeforeBindingDoesNotConnect','unrelatedSavedBindingUsesIndependentUsbMaintenance','multipleUsbDevicesRequireChoice','multipleNetworkInterfacesRequireExplicitMatching','disambiguatedBootstrapUsesChosenPhoneAndInterfaces','noSameSubnetHasActionAndBlocksBinding','lateReceiverStatusAfterNextDoesNotReopenWizard','receiverStatusFailureStaysOnStepTwo','receiverStatusRetryAdvancesOnlyAfterSuccess','receiverInterfacesFailureStaysOnStepTwo','receiverInterfacesRetryAdvancesOnlyAfterSuccess','deviceChangeInvalidatesPendingInterfaceCheck','closedWizardDoesNotCancelOrReopenCommittedInstall','largeTextWizardNoHorizontalOverflow','largeTextWizardCanScrollToPrimaryAction'])assert.equal(results[key],true,key);
  return;
 }
 if(process.env.ANPOD_WIRELESS_FLOW_ONLY){
  await page.setViewportSize({width:420,height:590});
  const dialog=page.getByRole('dialog',{name:'连接手机'});
  const oneInterface=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}];
  const oneCandidate=[{endpoint:'192.168.50.3:45678'},{endpoint:'192.168.50.4:45678'}];
  await page.evaluate(({interfaces,candidates})=>{window.reviewWirelessFlowConfigure(interfaces,['phoneA-abcdef'],candidates);window.reviewSetUdpReady();},{interfaces:oneInterface,candidates:oneCandidate});
  results.mainControlsKeepChineseAccessibleNames=await page.getByRole('button',{name:'连接手机',exact:true}).isVisible()&&await page.getByRole('group',{name:'连接方式'}).isVisible()&&await page.getByRole('group',{name:'播放模式'}).isVisible();
  await page.getByRole('button',{name:'打开设置',exact:true}).click();
  const settings=page.getByRole('dialog',{name:'设置'});
  results.settingsKeepChineseLabels=await settings.getByRole('checkbox',{name:/自动连接上次使用的手机/}).isVisible()&&await settings.getByLabel('无线协议').inputValue()==='udp'&&await settings.getByLabel('接收后端').inputValue()==='receiver'&&await settings.getByLabel('主题').inputValue()==='System';
  await settings.getByRole('button',{name:'关闭',exact:true}).click();
  results.listenerNotStartedAutomatically=await page.evaluate(()=>window.reviewListenerCalls.length===0);
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.length===1);
  results.singleClickStartsOnUniqueSelections=await page.evaluate(()=>window.reviewListenerCalls[0]?.localAddress==='192.168.50.2'&&window.reviewListenerCalls[0]?.bindingReference==='phoneA-abcdef'&&window.reviewListenerCalls[0]?.allowed===true);
  results.listenerStatusAndStopAction=await page.getByRole('button',{name:'停止接收',exact:true}).isEnabled()&&await page.evaluate(()=>window.reviewRead().wirelessConnectionOffers.automaticBinding==='phoneA-abcdef');
  results.listeningStatusAndStopLabel=await page.locator('.status-line').innerText().then(text=>text==='等待手机开始接收')&&await page.getByRole('button',{name:'停止接收',exact:true}).count()===1;
  await page.screenshot({path:`${out}/wireless-listening-auto.png`});
  await page.getByRole('button',{name:'停止接收',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.length===2);
  results.stopCancelsListener=await page.evaluate(()=>window.reviewListenerCalls[1]?.allowed===false&&!window.reviewRead().wirelessConnectionOffers.active);
  await page.evaluate(()=>window.reviewFailNextListener('模拟监听失败'));
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.getByText(/模拟监听失败/).waitFor();
  results.listenerFailureHasRetry=await page.getByRole('button',{name:'连接手机',exact:true}).isEnabled()&&await page.evaluate(()=>!window.reviewRead().wirelessConnectionOffers.active);
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.length===4&&window.reviewRead().wirelessConnectionOffers.active);
  results.listenerRetryUsesCorrectBinding=await page.evaluate(()=>window.reviewListenerCalls[3]?.localAddress==='192.168.50.2'&&window.reviewListenerCalls[3]?.bindingReference==='phoneA-abcdef');
  await page.evaluate(()=>window.reviewSetBusy());
  results.listenerCanStopWhileBackendBusy=await page.getByRole('button',{name:'停止接收',exact:true}).isEnabled();
  await page.getByRole('button',{name:'停止接收',exact:true}).click();
  results.listenerStopReachesBackendWhileBusy=await page.evaluate(()=>window.reviewListenerCalls.at(-1)?.allowed===false&&!window.reviewRead().wirelessConnectionOffers.active);
  await page.evaluate(()=>window.reviewSetBusy(false));

  const multipleInterfaces=[{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]},{index:2,name:'Ethernet',address:'192.168.50.3',prefixLength:24,gateways:[]}];
  const multipleCandidates=[{endpoint:'192.168.50.5:45678'},{endpoint:'192.168.50.6:45678'}];
  await page.evaluate(({interfaces,candidates})=>{window.reviewWirelessFlowConfigure(interfaces,['phoneA-abcdef','phoneB-abcdef'],candidates);},{interfaces:multipleInterfaces,candidates:multipleCandidates});
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await dialog.waitFor();
  await dialog.getByLabel('电脑网络接口').waitFor();
  await dialog.getByLabel('已配对手机').waitFor();
  results.multipleChoicesRequireSelection=await dialog.getByLabel('电脑网络接口').inputValue()===''&&await dialog.getByLabel('已配对手机').inputValue()==='';
  await dialog.getByLabel('电脑网络接口').selectOption('192.168.50.3');
  await dialog.getByLabel('已配对手机').selectOption('phoneB-abcdef');
  await dialog.getByRole('button',{name:'刷新接口和配对信息',exact:true}).click();
  await page.waitForTimeout(100);
  results.validSelectionsSurviveRefresh=await dialog.getByLabel('电脑网络接口').inputValue()==='192.168.50.3'&&await dialog.getByLabel('已配对手机').inputValue()==='phoneB-abcdef';

  const changedInterfaces=[{index:3,name:'Wi-Fi 2',address:'192.168.60.2',prefixLength:24,gateways:[]},{index:4,name:'Ethernet 2',address:'192.168.60.3',prefixLength:24,gateways:[]}];
  await page.evaluate(({interfaces,candidates})=>window.reviewWirelessFlowConfigure(interfaces,['phoneC-abcdef','phoneD-abcdef'],candidates),{interfaces:changedInterfaces,candidates:[{endpoint:'192.168.60.4:45678'},{endpoint:'192.168.60.5:45678'}]});
  await dialog.getByRole('button',{name:'刷新接口和配对信息',exact:true}).click();
  await page.waitForTimeout(100);
  results.invalidSelectionsAreRevoked=await dialog.getByLabel('电脑网络接口').inputValue()===''&&await dialog.getByLabel('已配对手机').inputValue()==='';
  await dialog.getByLabel('电脑网络接口').selectOption('192.168.60.2');
  await dialog.getByLabel('已配对手机').selectOption('phoneD-abcdef');
  await dialog.getByRole('button',{name:'刷新接口和配对信息',exact:true}).click();
  await page.waitForTimeout(100);
  results.userSelectionsSurviveRefresh=await dialog.getByLabel('电脑网络接口').inputValue()==='192.168.60.2'&&await dialog.getByLabel('已配对手机').inputValue()==='phoneD-abcdef';
  await dialog.getByRole('button',{name:'开始监听',exact:true}).click();
  results.listenerUsesExplicitChoices=await page.evaluate(()=>window.reviewListenerCalls.at(-1)?.localAddress==='192.168.60.2'&&window.reviewListenerCalls.at(-1)?.bindingReference==='phoneD-abcdef');
  await dialog.getByText(/手机点击“开始接收”后自动认证/).waitFor();
  results.automaticModeDoesNotOfferPerRequestApproval=await dialog.getByRole('button',{name:'接受并连接所选手机',exact:true}).count()===0;
  await page.evaluate(()=>window.reviewWirelessOffers([{id:'ambiguous-a',endpoint:'192.168.60.9:49772'},{id:'ambiguous-b',endpoint:'192.168.60.10:49772'}],true,'phoneD-abcdef'));
  results.multipleAutomaticTargetsRemainSelectable=await dialog.getByRole('button',{name:'接受并连接所选手机',exact:true}).isVisible();
  await dialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByRole('button',{name:'停止接收',exact:true}).click();

  await page.locator('.device-select').click();
  const manualDialog=page.getByRole('dialog',{name:'连接手机'});
  await manualDialog.waitFor();
  await manualDialog.getByLabel('已配对手机').selectOption('phoneD-abcdef');
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
  await page.getByRole('button',{name:'播放到手机',exact:true}).click();
  results.udpSessionComputerOutputCanPlay=await page.evaluate(()=>window.reviewCalls.set_output===1&&window.reviewRead().output==='phone');
  results.activeSessionDoesNotStartListener=await page.getByRole('button',{name:'停止',exact:true}).isEnabled()&&await page.evaluate(()=>window.reviewListenerCalls.filter(call=>call.allowed).length===4);
  fs.writeFileSync(`${out}/wireless-flow-result.json`,JSON.stringify(results,null,2));
  const {layouts,...summary}=results;
  console.log(JSON.stringify(summary));
  await browser.close();
  assert.deepEqual(errors,[]);
  for(const key of ['mainControlsKeepChineseAccessibleNames','settingsKeepChineseLabels','listenerNotStartedAutomatically','singleClickStartsOnUniqueSelections','listenerStatusAndStopAction','listeningStatusAndStopLabel','listenerCanStopWhileBackendBusy','listenerStopReachesBackendWhileBusy','stopCancelsListener','listenerFailureHasRetry','listenerRetryUsesCorrectBinding','multipleChoicesRequireSelection','validSelectionsSurviveRefresh','invalidSelectionsAreRevoked','userSelectionsSurviveRefresh','listenerUsesExplicitChoices','automaticModeDoesNotOfferPerRequestApproval','multipleAutomaticTargetsRemainSelectable','singleRequestPrefilledButNotAccepted','multiOfferPreservesStillValidChoice','expiredSelectionClearedWithoutAcceptance','explicitRequestSelectionSurvivesRefresh','onlyExplicitAcceptanceReachesBackend','udpSessionComputerOutputCanPlay','activeSessionDoesNotStartListener'])assert.equal(results[key],true,key);
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
  if(process.env.ANPOD_USB_RESET_ONLY){
   await page.setViewportSize({width:360,height:500});
   const openManager=async()=>{await page.getByRole('button',{name:'打开设置',exact:true}).click();await page.getByRole('button',{name:'手机应用',exact:true}).click();await page.getByRole('dialog',{name:'手机应用'}).waitFor();await page.locator('.pairing-repair-disclosure > summary').click();};
   const closeManager=async()=>{const current=page.getByRole('dialog',{name:'手机应用'});await current.getByRole('button',{name:'关闭',exact:true}).click();await current.waitFor({state:'detached'});};
   await page.evaluate(()=>window.reviewUsbScenario({bindings:['phoneA-abcdef','unrelated-phone-abcdef']}));await openManager();
    const dialog=page.getByRole('dialog',{name:'手机应用'});
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();
   await dialog.getByText('手机与 PC 保存的绑定摘要不同。').waitFor();
   results.previewDoesNotResetAndExplainsConflict=await page.evaluate(()=>window.reviewCalls.receiver_binding_reset_preview===1&&!window.reviewCalls.receiver_reset_binding);
   await page.screenshot({path:`${out}/usb-reset-preview.png`});
   await dialog.getByRole('button',{name:'确认清除此目标的两端记录'}).click();
   await dialog.getByText('两端绑定记录已清除').waitFor();
   results.confirmUsesCompletePreviewAndRefreshes=await page.evaluate(()=>{const args=window.reviewCommandArgs.receiver_reset_binding;return args.preview.serial==='TESTUSB'&&args.preview.androidUser==='0'&&args.preview.bindingReference==='phoneA-abcdef'&&args.preview.phoneCommitment==='a'.repeat(64)&&args.preview.pcCommitment==='b'.repeat(64)&&window.reviewCalls.receiver_binding_references>=2&&window.reviewCalls.refresh_devices>=1;});
   results.otherSavedBindingPreserved=await page.evaluate(()=>window.reviewCommandArgs.receiver_reset_binding.preview.bindingReference==='phoneA-abcdef'&&window.reviewCalls.receiver_reset_binding===1&&window.reviewSavedBindings().join(',')==='unrelated-phone-abcdef'&&!window.reviewCalls.set_output&&!window.reviewCalls.start_network_audio);
   await page.screenshot({path:`${out}/usb-reset-success.png`});await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({bindings:['phoneA-abcdef','unrelated-phone-abcdef'],resetPreview:{serial:'TESTUSB',androidUser:'10',bindingReference:'phoneA-abcdef',phoneState:'empty',phoneCommitment:null,pcState:'saved',pcCommitment:'b'.repeat(64),receiverBusy:false},resetResult:{outcome:'phoneClearedPcRetained',serial:'TESTUSB',androidUser:'10',bindingReference:'phoneA-abcdef',phoneState:'empty',phoneCommitment:null,pcState:'saved',pcCommitment:'b'.repeat(64),reason:'手机已清除；PC 记录仍存在。',nextAction:'重新预览后再重试。'}}));await openManager();
    await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByRole('button',{name:'确认清除此目标的两端记录'}).click();
   await dialog.getByRole('alert').filter({hasText:'手机已清除，PC 记录保留'}).waitFor();
   results.partialOutcomeIsNotReportedAsSuccess=await dialog.locator('.binding-reset-result.is-partial').count()===1&&await dialog.getByText('手机已清除；PC 记录仍存在。').count()===1;
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({resetPreviewError:'已安装 Receiver 不支持条件绑定重置；请先升级 Receiver。未停止会话或删除任何绑定。'}));await openManager();
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();
   await dialog.locator('.binding-reset-error details summary').click();
   results.unsupportedReceiverKeepsErrorAndNoReset=await dialog.getByRole('alert').innerText().then(text=>text.includes('不支持条件绑定重置'))&&await page.evaluate(()=>!window.reviewCalls.receiver_reset_binding);
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({devices:[{serial:'TESTUSB',transport:'usb',state:'unauthorized',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'未授权',connectable:false}]}));await openManager();
   results.unauthorizedDeviceCannotPreview=await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).isDisabled()&&await page.evaluate(()=>!window.reviewCalls.receiver_binding_reset_preview);await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({resetError:'重置预览已过期（设备、Android user 或绑定摘要发生变化）；未删除绑定，请重新检查。'}));await openManager();
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByRole('button',{name:'确认清除此目标的两端记录'}).click();
   await dialog.locator('.binding-reset-error details summary').click();
   results.stalePreviewErrorRemainsActionable=await dialog.getByRole('alert').innerText().then(text=>text.includes('预览已过期'))&&await page.evaluate(()=>window.reviewCalls.receiver_reset_binding===1);
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({}));await openManager();
   await page.waitForFunction(()=>{const button=[...document.querySelectorAll('.receiver-manager button')].find(value=>value.textContent==='检查状态');return !!button&&!button.disabled;});
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByText('只读预览 · TESTUSB').waitFor();
   await page.evaluate(()=>window.reviewDeferNextReceiverStatus());
   await dialog.locator('.binding-reset-actions').getByRole('button',{name:'重新检查'}).click();
   await page.waitForFunction(()=>typeof window.reviewResolveReceiverStatus==='function');
   results.recheckWaitsForReceiverOperationSlot=await page.evaluate(()=>window.reviewCalls.receiver_binding_reset_preview===1&&window.reviewManagementSlotConflicts()===0);
   await page.evaluate(()=>window.reviewResolveReceiverStatus());
   await page.waitForFunction(()=>window.reviewCalls.receiver_binding_reset_preview===2);
   results.recheckCompletesStatusBeforePreview=await page.evaluate(()=>window.reviewManagementSlotConflicts()===0&&window.reviewManagementSlotTrace.slice(-4).join(',')==='receiver_status:start,receiver_status:end,receiver_binding_reset_preview:start,receiver_binding_reset_preview:end');
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({}));await openManager();
   await page.waitForFunction(()=>{const button=[...document.querySelectorAll('.receiver-manager button')].find(value=>value.textContent==='检查状态');return !!button&&!button.disabled;});
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByText('只读预览 · TESTUSB').waitFor();
   await page.evaluate(()=>window.reviewDeferNextReceiverStatus());await dialog.locator('.binding-reset-actions').getByRole('button',{name:'重新检查'}).click();
   await page.waitForFunction(()=>typeof window.reviewResolveReceiverStatus==='function');await closeManager();await page.evaluate(()=>window.reviewResolveReceiverStatus());
   await page.waitForFunction(()=>!window.reviewManagementSlotBusy());
   results.closedRecheckDoesNotSendStalePreview=await page.evaluate(()=>window.reviewCalls.receiver_binding_reset_preview===1&&window.reviewManagementSlotConflicts()===0);
   await page.evaluate(()=>window.reviewUsbScenario({devices:[{serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true},{serial:'TESTSECOND',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8',androidVersion:'14',apiLevel:'34',displayName:'Google Pixel 8',statusText:'可连接',connectable:true}],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'},TESTSECOND:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));await openManager();
   await page.waitForFunction(()=>{const button=[...document.querySelectorAll('.receiver-manager button')].find(value=>value.textContent==='检查状态');return !!button&&!button.disabled;});
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByText('只读预览 · TESTUSB').waitFor();
   await page.evaluate(()=>window.reviewDeferNextReceiverStatus());await dialog.locator('.binding-reset-actions').getByRole('button',{name:'重新检查'}).click();
   await page.waitForFunction(()=>typeof window.reviewResolveReceiverStatus==='function');
   await page.evaluate(()=>window.reviewChangeUsbDevices([{serial:'TESTSECOND',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8',androidVersion:'14',apiLevel:'34',displayName:'Google Pixel 8',statusText:'可连接',connectable:true}]));
   await page.waitForFunction(()=>[...document.querySelectorAll('.receiver-usb-card select')].some(select=>select.value==='TESTSECOND'));
   await page.evaluate(()=>window.reviewResolveReceiverStatus());await page.waitForFunction(()=>!window.reviewManagementSlotBusy());
   results.changedTargetDoesNotSendStalePreview=await page.evaluate(()=>window.reviewCalls.receiver_binding_reset_preview===1);
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({resetPreview:{serial:'TESTUSB',androidUser:'10',bindingReference:'phoneA-abcdef',phoneState:'empty',phoneCommitment:null,pcState:'missing',pcCommitment:null,receiverBusy:false},resetResult:{outcome:'alreadyEmpty',serial:'TESTUSB',androidUser:'10',bindingReference:'phoneA-abcdef',phoneState:'empty',phoneCommitment:null,pcState:'missing',pcCommitment:null,reason:'兩端都沒有此目標的綁定。',nextAction:'如需配對，請單獨啟動 USB 首次綁定。'}}));await openManager();
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByRole('button',{name:'确认清除此目标的两端记录'}).click();
   await dialog.getByText('目标原本为空，未执行删除').waitFor();
   results.alreadyEmptyIsSuccessfulNoOp=await page.evaluate(()=>window.reviewCalls.receiver_reset_binding===1)&&await dialog.getByText('兩端都沒有此目標的綁定。').count()===1;
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({resetPreview:{serial:'TESTUSB',androidUser:'0',bindingReference:'phoneA-abcdef',phoneState:'unknown',phoneCommitment:null,pcState:'unknown',pcCommitment:null,receiverBusy:true}}));await openManager();
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();
   results.unknownAndBusyStateBlocksConfirmation=await dialog.getByRole('button',{name:'确认清除此目标的两端记录'}).isDisabled()&&await dialog.getByRole('alert').count()>=2;
   await closeManager();
   await page.evaluate(()=>{window.reviewUsbScenario({});window.reviewDeferNextReset();});await openManager();
   await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByRole('button',{name:'确认清除此目标的两端记录'}).click();
   await page.waitForFunction(()=>typeof window.reviewResolveReset==='function');
   await closeManager();await openManager();await page.evaluate(()=>window.reviewResolveReset());
   await page.waitForTimeout(50);
   results.closedWindowDoesNotResurrectResult=await dialog.getByText('两端绑定记录已清除').count()===0&&await page.evaluate(()=>window.reviewCalls.receiver_reset_binding===1);
   results.committedOperationWasNotCancelled=await page.evaluate(()=>!window.reviewCalls.cancel_operation);
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({}));await openManager();await dialog.getByRole('button',{name:'重置配对（需要 USB）'}).click();await dialog.getByText('只读预览 · TESTUSB').waitFor();
   await page.evaluate(()=>document.querySelector('.app-shell').classList.add('large-text'));
   const resetConfirm=dialog.getByRole('button',{name:'确认清除此目标的两端记录'});await resetConfirm.scrollIntoViewIfNeeded();
   results.largeTextPreviewRemainsScrollable=await dialog.evaluate(el=>el.scrollHeight>el.clientHeight&&el.scrollTop>0)&&await resetConfirm.isVisible()&&await page.evaluate(()=>document.documentElement.scrollWidth===innerWidth);
   await page.screenshot({path:`${out}/usb-reset-preview-bottom-large-text.png`});await page.keyboard.press('Escape');
   results.escapeClosesWithoutCommitting=await page.getByRole('dialog').count()===0&&await page.evaluate(()=>!window.reviewCalls.receiver_reset_binding);
   const resetChecks=['previewDoesNotResetAndExplainsConflict','confirmUsesCompletePreviewAndRefreshes','otherSavedBindingPreserved','partialOutcomeIsNotReportedAsSuccess','unsupportedReceiverKeepsErrorAndNoReset','unauthorizedDeviceCannotPreview','stalePreviewErrorRemainsActionable','recheckWaitsForReceiverOperationSlot','recheckCompletesStatusBeforePreview','closedRecheckDoesNotSendStalePreview','changedTargetDoesNotSendStalePreview','alreadyEmptyIsSuccessfulNoOp','unknownAndBusyStateBlocksConfirmation','closedWindowDoesNotResurrectResult','committedOperationWasNotCancelled','largeTextPreviewRemainsScrollable','escapeClosesWithoutCommitting'];
   for(const key of resetChecks)assert.equal(results[key],true,key);
   results.assertionCount=resetChecks.length;results.passedCount=resetChecks.filter(key=>results[key]===true).length;
   fs.writeFileSync(`${out}/usb-reset-result.json`,JSON.stringify(results,null,2));const {layouts,...summary}=results;console.log(JSON.stringify(summary));await browser.close();assert.deepEqual(errors,[]);
   return;
  }
  if(process.env.ANPOD_USB_UPGRADE_ONLY){
   await page.setViewportSize({width:420,height:590});
    const openManager=async()=>{await page.getByRole('button',{name:'打开设置',exact:true}).click();await page.getByRole('button',{name:'手机应用',exact:true}).click();await page.getByRole('dialog',{name:'手机应用'}).waitFor();};
   const closeManager=async()=>page.getByRole('dialog',{name:'手机应用'}).getByRole('button',{name:'关闭',exact:true}).click();
   await page.evaluate(()=>window.reviewReceiverVersionError());
   await page.getByRole('alert').filter({hasText:'手机端版本不符合要求，请通过 USB 检查并升级。'}).waitFor();
   results.versionMismatchShowsConciseRecovery=!(await page.locator('main').innerText()).includes('RECEIVER_UPDATE_REQUIRED:')&&await page.getByRole('button',{name:'播放到手机',exact:true}).count()===0;
   await page.screenshot({path:`${out}/version-upgrade-entry.png`});
   await page.getByRole('button',{name:'升级手机端',exact:true}).click();
   const upgradeManager=page.getByRole('dialog',{name:'手机应用'});
   await upgradeManager.waitFor();
   await upgradeManager.getByRole('button',{name:'更新手机应用',exact:true}).waitFor();
   results.versionMismatchOpensUsbUpgrade=await upgradeManager.getByLabel('已授权且在线的 USB 手机').inputValue()==='TESTUSB';
   await upgradeManager.getByRole('button',{name:'更新手机应用',exact:true}).click();
   await upgradeManager.locator('.receiver-version-details > summary').click();
   await upgradeManager.getByText(/USB 操作返回的已核验结果显示 TESTUSB 的版本码为 9/).waitFor();
   results.versionMismatchUpgradeUsesReturnedVerifiedStatus=await page.evaluate(()=>window.reviewCalls.receiver_install===1&&window.reviewCalls.receiver_status===1);
   await closeManager();
   assert.equal(results.versionMismatchShowsConciseRecovery,true);
   assert.equal(results.versionMismatchOpensUsbUpgrade,true);
   assert.equal(results.versionMismatchUpgradeUsesReturnedVerifiedStatus,true);
   await page.evaluate(()=>window.reviewUsbScenario({active:true,activeVersion:6,binding:'phoneA-abcdef',candidate:{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:6,bindingReference:'phoneA-abcdef'},statuses:{TESTUSB:{installed:true,versionName:'0.6.0',versionCode:6,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();
   await page.getByRole('button',{name:'更新手机应用',exact:true}).waitFor();
   results.managerDefaultIsCompact=await page.locator('.receiver-manager').count()===1&&!await page.locator('.receiver-version-details').evaluate(el=>el.open)&&!await page.locator('.pairing-repair-disclosure').evaluate(el=>el.open);
   results.sessionUpgradeBindingIsVisible=await page.getByLabel('选择当前会话对应的配对手机').isVisible();
   await page.screenshot({path:`${out}/receiver-manager-default.png`});
   await page.locator('.receiver-version-details > summary').click();
   results.legacyWithoutNetworkUpdateBitStillOffersUsbUpgrade=await page.locator('.receiver-usb-card').innerText().then(text=>text.includes('当前版本码 6')&&text.includes('目标版本码 9'));
   await page.getByRole('button',{name:'更新手机应用',exact:true}).click();
   results.activePlaybackRequiresSingleConfirmation=await page.getByRole('dialog').count()===1&&await page.getByRole('dialog',{name:'停止播放并更新手机应用？'}).count()===1;
   await page.getByRole('button',{name:'取消',exact:true}).click();
   await page.getByRole('dialog',{name:'手机应用'}).waitFor();
   await page.getByRole('button',{name:'更新手机应用',exact:true}).click();
   const statusChecksBeforeConfirmedUpgrade=await page.evaluate(()=>window.reviewCalls.receiver_status);
   await page.getByRole('button',{name:'停止播放并继续',exact:true}).click();
   if(!await page.locator('.receiver-version-details').evaluate(el=>el.open)) await page.locator('.receiver-version-details > summary').click();
   try { await page.getByText('USB 操作返回的已核验结果显示 TESTUSB 的版本码为 9。').waitFor({timeout:5000}); }
   catch(error) { console.log('USB upgrade failure page:', await page.locator('body').innerText()); await page.screenshot({path:`${out}/receiver-usb-upgrade-failure.png`}); throw error; }
   results.upgradeUsesSelectedBindingAndReturnedStatus=await page.evaluate(statusChecks=>window.reviewCommandArgs.receiver_upgrade_usb?.serial==='TESTUSB'&&window.reviewCommandArgs.receiver_upgrade_usb?.bindingReference==='phoneA-abcdef'&&!window.reviewCalls.receiver_install&&window.reviewCalls.receiver_status===statusChecks&&window.reviewCalls.refresh_devices===1,statusChecksBeforeConfirmedUpgrade);
   results.successToastIsNotFailure=await page.locator('.toast').innerText().then(text=>text.includes('Receiver 版本码 9 已核验；请重新认证连接。')&&!text.includes('操作没有完成'));
   results.successHasNoOldNetworkUpdateState=await page.locator('.status-line').innerText().then(text=>!text.includes('Receiver 更新'))&&await page.getByText('USB 操作返回的已核验结果显示 TESTUSB 的版本码为 9。').count()===1;
   await page.locator('.receiver-version-details > summary').click();
   await page.screenshot({path:`${out}/receiver-usb-upgrade.png`});
   await closeManager();

   for(const version of [9,10]){
    await page.evaluate(v=>window.reviewUsbScenario({candidate:{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},statuses:{TESTUSB:{installed:true,versionName:'existing',versionCode:v,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}),version);
    await openManager();await page.locator('.receiver-version-details > summary').click();await page.getByText('当前版本已达到目标版本，无需升级。').waitFor();
    results[version===9?'sameVersionDoesNotOfferUpgrade':'newerVersionDoesNotOfferDowngrade']=await page.getByRole('button',{name:/升级 Receiver ·/}).count()===0;
    await closeManager();
   }
   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:11,versionName:'0.9.1-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},statuses:{TESTUSB:{installed:true,versionName:'unknown',versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();await page.locator('.receiver-version-details > summary').click();await page.getByText(/待 USB 核验/).waitFor();
   results.unknownVersionDoesNotAssumeUpdate=await page.getByRole('button',{name:/升级 Receiver ·/}).count()===0;
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:11,versionName:'0.9.1-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},devices:[{...{serial:'TESTUSB',transport:'usb',state:'unauthorized',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'未授权',connectable:false}}],statuses:{}}));
   await openManager();
   results.noOrUnauthorizedUsbIsNotSelectable=await page.getByLabel('已授权且在线的 USB 手机').locator('option').count()===1&&await page.getByText('连接已授权的 USB 手机后可检查版本。').count()===1;
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({candidate:null,statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();
   results.invalidTargetDoesNotOfferOperation=await page.getByText('当前无法检查手机应用更新。').count()===1&&await page.getByRole('button',{name:/安装 Receiver ·|升级 Receiver ·/}).count()===0;
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:11,versionName:'0.9.1-usb',sizeBytes:5*1024*1024,currentVersionCode:8,bindingReference:'phoneA-abcdef'},bindings:['phoneA-abcdef','phoneB-abcdef'],active:true,binding:'phoneA-abcdef',activeVersion:8,devices:[{serial:'TESTUSB',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8 Pro',androidVersion:'15',apiLevel:'35',displayName:'Google Pixel 8 Pro',statusText:'可连接',connectable:true},{serial:'TESTSECOND',transport:'usb',state:'device',manufacturer:'Google',model:'Pixel 8',androidVersion:'14',apiLevel:'34',displayName:'Pixel 8',statusText:'可连接',connectable:true}],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'},TESTSECOND:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}},targetBindings:{TESTUSB:'phoneB-abcdef',TESTSECOND:'phoneB-abcdef'}}));
   await openManager();await page.getByRole('button',{name:/更新手机应用/}).waitFor();
   await page.getByLabel('已授权且在线的 USB 手机').selectOption('TESTSECOND');
   await page.getByRole('button',{name:'检查状态',exact:true}).waitFor();
   await page.waitForFunction(()=>!document.querySelector('dialog[open] .receiver-status-actions button')?.disabled);
   await page.getByRole('button',{name:'更新手机应用',exact:true}).click();

   await page.getByRole('button',{name:'停止播放并继续',exact:true}).click();
   await page.waitForFunction(()=>{const button=[...document.querySelectorAll('button')].find(value=>value.textContent?.trim()==='更新手机应用');return window.reviewCalls.receiver_upgrade_usb===1&&button&&!button.disabled;});
   results.wrongUsbTargetIsRejectedAndRetryRemains=await page.getByRole('button',{name:'更新手机应用',exact:true}).isVisible()&&await page.evaluate(()=>window.reviewGetUsbStatus('TESTSECOND').versionCode===8&&window.reviewCalls.receiver_upgrade_usb===1&&window.reviewRead().state==='streaming'&&window.reviewRead().sessionReceiverBinding==='phoneA-abcdef');
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:10,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:['unrelated-phone-abcdef'],statuses:{TESTUSB:{installed:true,versionName:'0.8.0',versionCode:8,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await openManager();await page.getByRole('button',{name:'更新手机应用',exact:true}).click();
   await page.locator('.receiver-version-details > summary').click();
   await page.getByText('USB 操作返回的已核验结果显示 TESTUSB 的版本码为 10。').waitFor();
   results.unrelatedSavedBindingDoesNotBlockUsbMaintenance=await page.evaluate(()=>window.reviewCommandArgs.receiver_install?.serial==='TESTUSB'&&!window.reviewCalls.receiver_upgrade_usb&&window.reviewCalls.receiver_status===1&&window.reviewCalls.refresh_devices===1);
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}},installError:'ADB 安装已取消'}));
   await openManager();await page.getByRole('button',{name:'安装手机应用',exact:true}).click();
   await page.getByText('连接没有完成。请检查所选手机和网络后重试。').waitFor();
   await page.locator('.network-error details summary').click();
   await page.getByText(/ADB 安装已取消/).waitFor();
   results.installFailureKeepsRetryAction=await page.getByRole('button',{name:'安装手机应用',exact:true}).isVisible()&&await page.evaluate(()=>window.reviewCalls.receiver_install===1&&window.reviewCalls.receiver_update_start===undefined);
   await closeManager();
   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:12,versionName:'0.9.2-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:[],statuses:{TESTUSB:{installed:false,versionName:null,versionCode:null,serviceActive:false,foreground:false,healthy:false,audioActive:false,error:null,message:'Receiver 未安装'}}}));
   await openManager();await page.getByRole('button',{name:'安装手机应用',exact:true}).click();
   await page.locator('.receiver-version-details > summary').click();
   await page.getByText('USB 操作返回的已核验结果显示 TESTUSB 的版本码为 12。').waitFor();
   results.installUsesReturnedVerifiedStatusAndRefreshes=await page.evaluate(()=>window.reviewCommandArgs.receiver_install?.serial==='TESTUSB'&&window.reviewCalls.receiver_status===1&&window.reviewCalls.refresh_devices===1);
   await closeManager();

   await page.evaluate(()=>window.reviewUsbScenario({candidate:{versionCode:9,versionName:'0.9.0-usb',sizeBytes:5*1024*1024,currentVersionCode:null,bindingReference:null},bindings:['unrelated-phone-abcdef'],statuses:{TESTUSB:{installed:true,versionName:'0.6.0',versionCode:6,serviceActive:false,foreground:false,healthy:true,audioActive:false,error:null,message:'Receiver 已安装'}}}));
   await page.evaluate(()=>window.reviewUsbWirelessReady());
   await page.locator('.device-select').click();
    const networkDialog=page.getByRole('dialog',{name:'连接手机'});
   await networkDialog.waitFor();
    const bootstrapMethod=networkDialog.getByRole('button',{name:'首次设置（USB）',exact:true});
   await bootstrapMethod.click();
    try { await page.waitForFunction(()=>{const dialog=document.querySelector('dialog[open][aria-label="连接手机"]');return dialog?.querySelector('.network-connect')?.disabled===false;},{timeout:5000}); }
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
   for(const key of ['managerDefaultIsCompact','sessionUpgradeBindingIsVisible','legacyWithoutNetworkUpdateBitStillOffersUsbUpgrade','activePlaybackRequiresSingleConfirmation','upgradeUsesSelectedBindingAndReturnedStatus','successToastIsNotFailure','successHasNoOldNetworkUpdateState','sameVersionDoesNotOfferUpgrade','newerVersionDoesNotOfferDowngrade','unknownVersionDoesNotAssumeUpdate','noOrUnauthorizedUsbIsNotSelectable','invalidTargetDoesNotOfferOperation','wrongUsbTargetIsRejectedAndRetryRemains','unrelatedSavedBindingDoesNotBlockUsbMaintenance','installFailureKeepsRetryAction','installUsesReturnedVerifiedStatusAndRefreshes','compatibleOldReceiverCanConnectWithOptionalUpgrade','oldReceiverStartsFirstBindWithoutUpgrade'])assert.equal(results[key],true,key);
  return;
 }
 await page.setViewportSize({width:420,height:590});await page.evaluate(()=>window.reviewSet('ready','wifi'));
 await page.locator('.device-select').click();
 await page.getByRole('dialog',{name:'选择手机'}).getByRole('button',{name:'添加手机',exact:true}).click();
 const pairDialog=page.getByRole('dialog',{name:'无线 ADB 兼容连接'});
 await page.getByRole('tab',{name:'配对码',exact:true}).click();
 results.pairDialogTabsKeepChineseAccessibleNames=await pairDialog.getByRole('tab',{name:'扫码配对',exact:true}).count()===1&&await pairDialog.getByRole('tab',{name:'手动地址',exact:true}).count()===1;
 const code=page.getByLabel('6 位配对码');await code.fill('123456');await code.focus();
 await page.evaluate(()=>window.reviewEmit());await page.waitForTimeout(100);
 results.focusStable=await code.evaluate(el=>document.activeElement===el);
 results.codeRetained=await code.inputValue();
 await page.screenshot({path:`${out}/pair-code.png`});
 results.modalKeepsScrollingWithoutScrollbar=await pairDialog.evaluate(el=>{const oldHeight=el.style.height,oldMaxHeight=el.style.maxHeight;el.style.height='180px';el.style.maxHeight='none';el.scrollTop=120;const hidden=getComputedStyle(el).scrollbarWidth==='none'||getComputedStyle(el,'::-webkit-scrollbar').display==='none';const result=el.scrollHeight>el.clientHeight&&getComputedStyle(el).overflowY==='auto'&&hidden&&el.scrollTop>0;el.style.height=oldHeight;el.style.maxHeight=oldMaxHeight;return result;});
 await pairDialog.getByRole('tab',{name:'扫码配对',exact:true}).click();
 await pairDialog.getByRole('button',{name:'开始扫码配对',exact:true}).waitFor();
 await pairDialog.getByRole('button',{name:'开始扫码配对',exact:true}).click();
 await page.waitForFunction(()=>window.reviewCalls.pair_qr===1);
 await pairDialog.getByRole('button',{name:'取消配对',exact:true}).click();
 results.qrCancellationUsesBackendAndLeavesPairCode=await pairDialog.locator('.pair-progress').innerText()==='配对已取消。'&&await page.evaluate(()=>window.reviewCalls.cancel_qr_pairing>=2&&window.reviewCalls.pair_qr===1);
 await pairDialog.getByRole('tab',{name:'配对码',exact:true}).click();
 results.pairCodeSurvivesQrCancel=await code.inputValue()==='123456';
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
  await page.evaluate(()=>{window.reviewCalls={};window.reviewCommandArgs={};});
  await page.evaluate(()=>window.reviewSet('ready','usb'));
  await page.locator('.device-select').click();
  const deviceDialog=page.getByRole('dialog',{name:'选择手机'});
  results.deviceDialogChineseLabels=await deviceDialog.getByText('只显示当前连接方式的设备').isVisible()&&await deviceDialog.getByRole('button',{name:'刷新',exact:true}).isVisible()&&await deviceDialog.getByRole('button',{name:'关闭',exact:true}).isVisible();
  await page.locator('.device-option').nth(1).click();
  results.selectDoesNotConnect=await page.evaluate(()=>window.reviewRead().selectedSerial==='TESTSECOND'&&!window.reviewRead().sessionSerial&&!(window.reviewCalls.connect_device_only||0));
  await page.getByRole('button',{name:'播放到手机',exact:true}).click();
  await page.getByRole('button',{name:'停止',exact:true}).waitFor();
  results.usbPrimaryStartsPlayback=await page.evaluate(()=>window.reviewCalls.connect_device===1&&window.reviewCommandArgs.connect_device?.serial==='TESTSECOND'&&window.reviewRead().output==='phone');
  await page.getByRole('button',{name:'停止',exact:true}).click();
  await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
  results.stopRestoresComputer=await page.evaluate(()=>window.reviewCalls.disconnect===1&&window.reviewRead().output==='computer');
  await page.evaluate(()=>{window.reviewCalls={};window.reviewCommandArgs={};});
  await page.evaluate(()=>window.reviewSet('connected','usb'));
  await page.getByRole('button',{name:'播放到手机',exact:true}).click();
  results.activeComputerOutputStartsWithoutDisconnect=await page.evaluate(()=>window.reviewCalls.set_output===1&&window.reviewRead().output==='phone'&&!window.reviewCalls.disconnect);
  await page.getByRole('button',{name:'切回电脑',exact:true}).click();
  results.secondaryComputerOutputWorks=await page.evaluate(()=>window.reviewRead().output==='computer'&&window.reviewCalls.set_output===2);
  await page.getByRole('button',{name:'播放到手机',exact:true}).click();
  await page.getByRole('button',{name:'停止',exact:true}).click();
  await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
  await page.evaluate(()=>{window.reviewCalls={};window.reviewCommandArgs={};});
  await page.evaluate(()=>window.reviewSet('ready','wifi'));
  await page.locator('.device-select').click();await page.locator('.device-option').nth(0).click();
  await page.getByRole('button',{name:'播放到手机',exact:true}).click();
  await page.getByRole('button',{name:'停止',exact:true}).waitFor();
  results.adbPrimaryStartsPlayback=await page.evaluate(()=>window.reviewCalls.connect_device===1&&window.reviewCommandArgs.connect_device?.serial==='TESTUSB'&&window.reviewRead().output==='phone');
  await page.getByRole('button',{name:'停止',exact:true}).click();
  await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
  await page.evaluate(()=>window.reviewSet('streaming','usb'));
 await page.getByRole('button',{name:'低延迟',exact:true}).click();
 const confirmDialog=page.getByRole('dialog',{name:'切换播放模式？'});
 results.modeNeedsConfirmation=await confirmDialog.isVisible()&&await confirmDialog.getByRole('button',{name:'取消',exact:true}).isVisible()&&await confirmDialog.getByRole('button',{name:'重新配置',exact:true}).isVisible();
 await page.getByRole('button',{name:'取消',exact:true}).click();
 results.cancelKeepsMode=await page.getByRole('button',{name:'均衡',exact:true}).getAttribute('aria-pressed')==='true';
  await page.evaluate(()=>window.reviewSetBusy());
  results.stopRemainsEnabledWhileBusy=await page.getByRole('button',{name:'停止',exact:true}).isEnabled();
  await page.getByRole('button',{name:'停止',exact:true}).click();
  await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
  results.stopDuringBusyDisconnects=await page.evaluate(()=>window.reviewCalls.disconnect===2);
  await page.evaluate(()=>{window.reviewCalls={};window.reviewSetActiveError();});
  results.activeErrorKeepsDisconnectRetry=await page.getByRole('button',{name:'重试断开',exact:true}).isEnabled()&&await page.locator('.status-description').innerText().then(text=>text.includes('连接还未完全断开')&&!text.includes('模拟播放失败详情'));
  await page.locator('.status-description details summary').click();
  results.errorDetailsRemainAvailableOnDemand=await page.locator('.status-description').innerText().then(text=>text.includes('模拟播放失败详情')&&text.includes('电脑声音保持输出'));
  await page.getByRole('button',{name:'重试断开',exact:true}).click();
  await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
  await page.evaluate(()=>{window.reviewCalls={};window.reviewCommandArgs={};});
  await page.evaluate(()=>window.reviewSetUdpStreaming());
  await page.setViewportSize({width:360,height:500});
  const compactCard=await page.evaluate(()=>{const card=document.querySelector('.device-card'),copy=card.querySelector('.device-copy'),actions=card.querySelector('.device-actions'),buttons=[...actions.querySelectorAll('button')];copy.querySelector('strong').textContent='paired-receiver-tag-that-is-deliberately-long';const rects=buttons.map(b=>{const r=b.getBoundingClientRect();return {top:r.top,height:r.height,left:r.left,right:r.right};});const cr=card.getBoundingClientRect(),xr=copy.getBoundingClientRect();return {buttonCount:buttons.length,aligned:rects.length===1&&Math.abs(rects[0].top-(cr.top+(cr.height-rects[0].height)/2))<2,touchTargets:rects.every(r=>r.height>=40),insideViewport:cr.left>=0&&cr.right<=innerWidth,identityDoesNotOverlapActions:rects.length===1&&xr.right<=rects[0].left,tagWraps:copy.scrollWidth<=copy.clientWidth};});
  results.activeUdpCompactPhoneCard=compactCard.buttonCount===1&&compactCard.aligned&&compactCard.touchTargets&&compactCard.insideViewport&&compactCard.identityDoesNotOverlapActions&&compactCard.tagWraps;
  await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');await page.screenshot({path:`${out}/udp-active-360x500.png`});
  await page.emulateMedia({colorScheme:'light',reducedMotion:'reduce'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='light');
  await page.setViewportSize({width:420,height:590});
  await page.getByRole('button',{name:'低延迟',exact:true}).click();
  results.udpModeReconfiguresWithoutConfirm=await page.locator('dialog[open]').count()===0&&await page.evaluate(()=>window.reviewCalls.reconfigure_session===1&&window.reviewRead().settings.playbackMode==='LowLatency');
  await page.evaluate(()=>window.reviewSetManagementOnly());
  results.managementSessionDoesNotOfferPlayback=await page.getByRole('button',{name:'断开管理连接',exact:true}).isEnabled()&&await page.getByRole('button',{name:'播放到手机',exact:true}).count()===0&&await page.locator('.status-line').innerText().then(text=>text.includes('暂不支持播放'));
  await page.getByRole('button',{name:'断开管理连接',exact:true}).click();
  await page.waitForFunction(()=>!window.reviewRead().canDisconnect);
  await page.evaluate(()=>window.reviewSet('noDevice','usb'));
  await page.getByRole('button',{name:'连接 USB 手机',exact:true}).click();
  const helpDialog=page.getByRole('dialog',{name:'连接帮助'});
  results.noTargetOffersHelp=await helpDialog.isVisible()&&await helpDialog.getByText('USB 默认使用 Receiver；已保存的选择不变。Legacy 可在设置的“高级连接”中手动选择。').isVisible()&&await helpDialog.getByRole('button',{name:'完成',exact:true}).isVisible();
  await helpDialog.getByRole('button',{name:'完成',exact:true}).click();
  await page.evaluate(()=>window.reviewSetUdpReady());
  await page.getByRole('button',{name:'连接帮助',exact:true}).click();
  const udpHelpDialog=page.getByRole('dialog',{name:'连接帮助'});
  results.udpHelpDescribesPhoneRestartAndUsbCheck=await udpHelpDialog.getByText(/重新打开手机上的 AnPod 并点击“开始接收”/).isVisible()&&await udpHelpDialog.getByText(/如仍无法连接，可通过 USB 检查 Receiver/).isVisible()&&!await udpHelpDialog.getByText(/需要通过 USB 重新启动/).count();
  await udpHelpDialog.getByRole('button',{name:'完成',exact:true}).click();
  await page.evaluate(()=>{window.reviewWirelessFlowConfigure([{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}],['phoneA-abcdef'],[{endpoint:'192.168.50.3:45678'}]);window.reviewSet('ready','usb');});
  await page.getByRole('button',{name:'无线',exact:true}).click();
  results.udpSelectionStaysOnMainScreen=await page.locator('dialog[open]').count()===0&&await page.getByRole('button',{name:'连接手机',exact:true}).isEnabled()&&await page.getByText('电脑会尝试连接所选手机；手机尚未允许时会先询问。').isVisible();
  results.udpPrimaryActionIsVerb=results.udpSelectionStaysOnMainScreen;
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===1);
  results.udpPrimaryConnectsOneDiscoveredTargetWithBackgroundAuth=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.backgroundAuthorizationRequired===true&&window.reviewCommandArgs.start_network_audio?.bindingReference==='phoneA-abcdef'&&window.reviewCommandArgs.start_network_audio?.localAddress==='192.168.50.2'&&window.reviewCommandArgs.start_network_audio?.peer==='192.168.50.3:45678'&&!window.reviewCalls.query_background_trust);

  await page.evaluate(()=>window.reviewWirelessFlowConfigure([{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}],['phoneA-abcdef'],[{endpoint:'192.168.50.3:45678'},{endpoint:'192.168.50.4:45678'}]));
  await page.getByRole('button',{name:'打开设置',exact:true}).click();

   await page.getByRole('button',{name:'手机应用',exact:true}).click();
   const trustDialog=page.getByRole('dialog',{name:'手机应用'});
  await page.locator('.background-trust-disclosure > summary').click();
  await trustDialog.getByRole('button',{name:'查看授权状态',exact:true}).click();
   await trustDialog.getByText('允许后台连接，直到取消').waitFor();
  await trustDialog.evaluate(el=>el.scrollTop=el.scrollHeight);
  await page.screenshot({path:`${out}/background-trust-status.png`});
  results.backgroundTrustQueryUsesActiveSessionGeneration=await page.evaluate(()=>window.reviewCommandArgs.query_background_trust?.target?.target==='activeSession'&&window.reviewCommandArgs.query_background_trust?.target?.sessionGeneration===41&&window.reviewCommandArgs.query_background_trust?.target?.bindingReference==='phoneA-abcdef');
  await page.evaluate(()=>window.reviewBackgroundTrust({kind:'untilRevoked',persistence:'binding',remainingMs:null},'模拟查询失败'));
  await trustDialog.getByRole('button',{name:'查看授权状态',exact:true}).click();
  await trustDialog.getByRole('alert').filter({hasText:'授权状态未知'}).waitFor();
  results.failedTrustQueryIsUnknown=await trustDialog.getByText('未授权 · 无持久化').count()===0;
  await page.evaluate(()=>{window.reviewBackgroundTrust({kind:'once',persistence:'process',remainingMs:12000});window.reviewDeferBackgroundTrust();});
  const queryCountBeforeStale=await page.evaluate(()=>window.reviewCalls.query_background_trust||0);
  await trustDialog.getByRole('button',{name:'查看授权状态',exact:true}).click();
  await page.waitForFunction(count=>(window.reviewCalls.query_background_trust||0)>count,queryCountBeforeStale);
  await page.evaluate(()=>{window.reviewTrustSessionGeneration(42);window.reviewResolveBackgroundTrust();});
  await page.waitForTimeout(50);
  results.staleTrustResponseDiscarded=await trustDialog.locator('.background-trust').innerText().then(text=>!text.includes('仅允许本次后台连接'))&&await trustDialog.getByRole('button',{name:'查看授权状态',exact:true}).isEnabled();
  await trustDialog.getByRole('button',{name:'取消授权',exact:true}).click();
  results.revokeNeedsExplicitTargetConfirmation=await page.evaluate(()=>!window.reviewCalls.revoke_background_trust)&&await trustDialog.locator('.background-trust-confirm').innerText().then(text=>text.includes('phoneA-a')&&text.includes('当前连接'));
  await trustDialog.evaluate(el=>el.scrollTop=el.scrollHeight);
  await page.screenshot({path:`${out}/background-trust-revoke-confirmation.png`});
  await trustDialog.getByRole('button',{name:'确认取消',exact:true}).click();
  await trustDialog.getByText('未授权 · 不保存').waitFor();
  await page.screenshot({path:`${out}/background-trust-verified-recent.png`});
  results.revokeUsesAckStateAndPreservesBinding=await page.evaluate(()=>window.reviewCommandArgs.revoke_background_trust?.target?.sessionGeneration===42&&window.reviewRead().sessionReceiverBinding===null&&window.reviewSavedBindings().includes('phoneA-abcdef'));
  await page.evaluate(()=>{window.reviewSetUdpReady();window.reviewWirelessFlowConfigure([{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}],['phoneA-abcdef'],[{endpoint:'192.168.50.3:45678'},{endpoint:'192.168.50.4:45678'}]);});
  const trustQueriesBeforeMulti=await page.evaluate(()=>window.reviewCalls.query_background_trust||0);
  await trustDialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  const multiTargetDialog=page.getByRole('dialog',{name:'连接手机'});
  await multiTargetDialog.getByLabel('手机地址').waitFor();
  results.multipleDiscoveredTargetsRequireChoice=await multiTargetDialog.getByLabel('手机地址').inputValue()===''&&await multiTargetDialog.getByRole('button',{name:'连接手机',exact:true}).isDisabled()&&await page.evaluate(()=>window.reviewCalls.start_network_audio===1);
  await multiTargetDialog.getByRole('button',{name:/192\.168\.50\.4:45678/}).click();
  await multiTargetDialog.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===2);
  results.explicitCandidateConnectsWithBackgroundAuth=await page.evaluate(before=>window.reviewCommandArgs.start_network_audio?.backgroundAuthorizationRequired===true&&window.reviewCommandArgs.start_network_audio?.peer==='192.168.50.4:45678'&&(window.reviewCalls.query_background_trust||0)===before,trustQueriesBeforeMulti);

  await page.evaluate(()=>{window.reviewSetUdpReady();window.reviewWirelessFlowConfigure([{index:2,name:'Ethernet',address:'10.0.0.2',prefixLength:24,gateways:[]}],['phoneB-abcdef'],[{endpoint:'10.0.0.3:45678'}]);});
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  const staleDirectTargetDialog=page.getByRole('dialog',{name:'连接手机'});
  results.replacedDirectTargetsRequireExplicitChoice=await staleDirectTargetDialog.getByLabel('电脑网络接口').inputValue()===''&&await staleDirectTargetDialog.getByLabel('已配对手机').inputValue()===''&&await page.evaluate(()=>window.reviewCalls.start_network_audio===2);
  await staleDirectTargetDialog.getByLabel('电脑网络接口').selectOption('10.0.0.2');
  await staleDirectTargetDialog.getByLabel('已配对手机').selectOption('phoneB-abcdef');
  await staleDirectTargetDialog.getByRole('button',{name:/10\.0\.0\.3:45678/}).click();
  await staleDirectTargetDialog.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.start_network_audio===3);
  results.explicitReplacementTargetConnects=await page.evaluate(()=>window.reviewCommandArgs.start_network_audio?.localAddress==='10.0.0.2'&&window.reviewCommandArgs.start_network_audio?.bindingReference==='phoneB-abcdef'&&window.reviewCommandArgs.start_network_audio?.backgroundAuthorizationRequired===true);

  await page.evaluate(()=>{window.reviewSetUdpReady();window.reviewWirelessFlowConfigure([{index:2,name:'Ethernet',address:'10.0.0.2',prefixLength:24,gateways:[]}],['phoneB-abcdef'],[{endpoint:'10.0.0.3:45678'}]);window.reviewBackgroundStartFailure('Receiver 不支持后台启动授权。');});
  const trustQueriesBeforeFailure=await page.evaluate(()=>window.reviewCalls.query_background_trust||0);
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.getByRole('dialog',{name:'连接手机'}).getByRole('alert').filter({hasText:'连接没有完成'}).waitFor();
  const fallbackDialog=page.getByRole('dialog',{name:'连接手机'});
  await fallbackDialog.getByRole('button',{name:'开始监听',exact:true}).click();
  await page.waitForFunction(()=>window.reviewListenerCalls.at(-1)?.allowed===true);
  results.backgroundAuthFailureKeepsManualOfferPath=await page.evaluate(before=>window.reviewCommandArgs.start_network_audio?.backgroundAuthorizationRequired===true&&window.reviewListenerCalls.at(-1)?.allowed===true&&(window.reviewCalls.query_background_trust||0)===before,trustQueriesBeforeFailure);
  await fallbackDialog.getByRole('button',{name:'关闭',exact:true}).click();
  await page.getByRole('button',{name:'停止接收',exact:true}).click();
  await page.waitForFunction(()=>window.reviewCalls.set_wireless_connection_offers>=2);
  results.udpListenerCanStop=await page.evaluate(()=>window.reviewListenerCalls.at(-1)?.allowed===false);

  await page.evaluate(()=>{window.reviewSetUdpReady();window.reviewWirelessFlowConfigure([{index:1,name:'Wi-Fi',address:'192.168.50.2',prefixLength:24,gateways:[]}],[],[]);});
  const startsBeforeNoBinding=await page.evaluate(()=>window.reviewCalls.start_network_audio||0);
  await page.getByRole('button',{name:'连接手机',exact:true}).click();
  await page.getByRole('dialog',{name:'首次设置'}).waitFor();
  results.noBindingKeepsUsbSetupAvailable=await page.getByRole('dialog',{name:'首次设置'}).isVisible()&&await page.evaluate(before=>(window.reviewCalls.start_network_audio||0)===before&&window.reviewRead().sessionConnectionTransport===null,startsBeforeNoBinding);
  results.noBindingStartsFirstSetup=results.noBindingKeepsUsbSetupAvailable;
  await page.getByRole('dialog',{name:'首次设置'}).getByRole('button',{name:'关闭',exact:true}).click();
 await page.evaluate(()=>window.reviewSet('connecting','wifi'));
 results.connectingCanBeCancelled=await page.getByRole('button',{name:'取消连接',exact:true}).isEnabled();
 results.busyBlocksModes=await page.getByRole('button',{name:'低延迟',exact:true}).isDisabled();
 await page.getByRole('button',{name:'取消连接',exact:true}).click();
 await page.waitForFunction(()=>window.reviewCalls.cancel_operation===1);
  results.cancelConnectionWorks=await page.getByRole('button',{name:'播放到手机',exact:true}).isEnabled();
 await page.evaluate(()=>window.reviewRecoverySeconds(7));
 results.recoveryCountdownUsesSnapshot=await page.locator('.status-line').innerText().then(text=>text.includes('剩余 7 秒'));
 await page.evaluate(()=>window.reviewRecoverySeconds(null));
 results.recoveryCountdownHiddenWhenAbsent=await page.locator('.status-line').innerText().then(text=>text==='正在恢复连接'&&!text.includes('剩余'));
 await page.evaluate(()=>window.reviewSet('streaming','usb'));
  await page.getByRole('button',{name:'打开设置',exact:true}).click();
  results.advancedOptionsAlwaysVisible=await page.getByRole('heading',{name:'高级连接',exact:true}).isVisible();
  results.advancedOptionsInSettings=await page.getByLabel('无线兼容方式').isVisible()&&await page.getByLabel('手机接收方式').isVisible();
 await page.getByRole('checkbox',{name:/放大文字/}).check();
 await page.locator('dialog').getByRole('button',{name:'关闭',exact:true}).click();
  await page.setViewportSize({width:420,height:590});
  results.standardViewportNoOverflow=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('.device-card').getBoundingClientRect().height>=document.querySelector('.device-copy').getBoundingClientRect().height);
  await page.screenshot({path:`${out}/main-420x590.png`});
  await page.setViewportSize({width:360,height:500});await page.emulateMedia({colorScheme:'dark',reducedMotion:'reduce'});await page.waitForFunction(()=>document.documentElement.dataset.theme==='dark');
 results.largeTextNoOverflow=await page.evaluate(()=>document.documentElement.scrollWidth<=innerWidth&&document.querySelector('.device-card').getBoundingClientRect().height>=document.querySelector('.device-copy').getBoundingClientRect().height);
  results.largeTextScrollGeometry=await page.evaluate(()=>{const main=document.querySelector('.main-scroll').getBoundingClientRect(),footer=document.querySelector('.footer').getBoundingClientRect(),scroll=document.querySelector('.main-scroll');return {mainBottom:main.bottom,footerTop:footer.top,scrollHeight:scroll.scrollHeight,clientHeight:scroll.clientHeight};});
  results.largeTextFitsOrCanScroll=results.largeTextScrollGeometry.footerTop>=results.largeTextScrollGeometry.mainBottom&&await page.evaluate(()=>{const el=document.querySelector('.main-scroll');if(el.scrollHeight<=el.clientHeight)return true;el.scrollTop=el.scrollHeight;return el.scrollTop>0;});
 await page.screenshot({path:`${out}/large-dark.png`});
 fs.writeFileSync(`${out}/result.json`,JSON.stringify(results,null,2));
 const {layouts,...summary}=results;
 console.log(JSON.stringify({...summary,layoutFailures:layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight)}));
 await browser.close();
 assert.deepEqual(errors,[]);
 if(!process.env.ANPOD_SKIP_LAYOUT_MATRIX)assert.equal(layouts.filter(x=>x.horizontalOverflow||x.deviceHeight<x.deviceTextHeight||x.width!==x.size[0]).length,0);
 for(const key of ['focusStable','pairDialogTabsKeepChineseAccessibleNames','modalKeepsScrollingWithoutScrollbar','qrCancellationUsesBackendAndLeavesPairCode','pairCodeSurvivesQrCancel','stalePairStaysClosed','cleanupRetryEnabled','cleanupRetryWorks','deviceDialogChineseLabels','selectDoesNotConnect','usbPrimaryStartsPlayback','stopRestoresComputer','activeComputerOutputStartsWithoutDisconnect','secondaryComputerOutputWorks','adbPrimaryStartsPlayback','modeNeedsConfirmation','cancelKeepsMode','stopRemainsEnabledWhileBusy','stopDuringBusyDisconnects','activeErrorKeepsDisconnectRetry','errorDetailsRemainAvailableOnDemand','activeUdpCompactPhoneCard','udpModeReconfiguresWithoutConfirm','managementSessionDoesNotOfferPlayback','noTargetOffersHelp','udpHelpDescribesPhoneRestartAndUsbCheck','udpSelectionStaysOnMainScreen','udpPrimaryActionIsVerb','udpPrimaryConnectsOneDiscoveredTargetWithBackgroundAuth','backgroundTrustQueryUsesActiveSessionGeneration','revokeNeedsExplicitTargetConfirmation','revokeUsesAckStateAndPreservesBinding','failedTrustQueryIsUnknown','staleTrustResponseDiscarded','multipleDiscoveredTargetsRequireChoice','explicitCandidateConnectsWithBackgroundAuth','replacedDirectTargetsRequireExplicitChoice','explicitReplacementTargetConnects','backgroundAuthFailureKeepsManualOfferPath','udpListenerCanStop','noBindingKeepsUsbSetupAvailable','noBindingStartsFirstSetup','connectingCanBeCancelled','busyBlocksModes','cancelConnectionWorks','recoveryCountdownUsesSnapshot','recoveryCountdownHiddenWhenAbsent','advancedOptionsAlwaysVisible','advancedOptionsInSettings','standardViewportNoOverflow','largeTextNoOverflow','largeTextFitsOrCanScroll'])assert.equal(results[key],true,key);
 assert.equal(results.codeRetained,'123456');
})().catch(e=>{console.error(e);process.exit(1)});
