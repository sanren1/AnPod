import coreSelfTest from './CoreSelfTest.test';
import hdcLoopbackTest from './HdcLoopback.test';
import entryParamsTest from './EntryParams.test';
import stringsParityTest from './StringsParity.test';

export default function testsuite() {
  coreSelfTest();
  hdcLoopbackTest();
  entryParamsTest();
  stringsParityTest();
}
