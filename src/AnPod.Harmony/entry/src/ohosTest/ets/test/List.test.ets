import aodDisplayTest from './AodDisplay.test';
import aodSettingsTest from './AodSettings.test';
import coreSelfTest from './CoreSelfTest.test';
import materialTest from './Material.test';
import hdcLoopbackTest from './HdcLoopback.test';
import entryParamsTest from './EntryParams.test';
import sheetLayoutTest from './SheetLayout.test';
import stringsParityTest from './StringsParity.test';

export default function testsuite() {
  aodDisplayTest();
  aodSettingsTest();
  coreSelfTest();
  hdcLoopbackTest();
  entryParamsTest();
  stringsParityTest();
  sheetLayoutTest();
  materialTest();
}
