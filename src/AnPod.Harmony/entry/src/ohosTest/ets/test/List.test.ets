import coreSelfTest from './CoreSelfTest.test';
import entryParamsTest from './EntryParams.test';
import stringsParityTest from './StringsParity.test';

export default function testsuite() {
  coreSelfTest();
  entryParamsTest();
  stringsParityTest();
}
