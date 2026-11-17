import aodDisplayTest from './AodDisplay.test';
import aodSettingsTest from './AodSettings.test';
import coreSelfTest from './CoreSelfTest.test';
import hdcLoopbackTest from './HdcLoopback.test';
import entryParamsTest from './EntryParams.test';
import stringsParityTest from './StringsParity.test';

export default function testsuite() {
  aodDisplayTest();
  aodSettingsTest();
  coreSelfTest();
  hdcLoopbackTest();
  entryParamsTest();
  stringsParityTest();
}
