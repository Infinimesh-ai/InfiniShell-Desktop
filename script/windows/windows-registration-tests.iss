[Setup]
AppId=InfiniShellRegistryTest-{#TestId}
AppName=InfiniShell Registry Test
AppVersion=1
DefaultDirName={tmp}\unused
CreateAppDir=no
Uninstallable=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputBaseFilename=registration-test
OutputDir={#EvidenceDir}
Compression=none

#include "windows-registration.iss"

[Code]
const
  TestRoot = 'Software\InfiniShellRegistryTests\{#TestId}';
  AppName = 'InfiniShell';
  AppId = 'dev.infinishell.InfiniShell';
  ExePath = 'C:\安装 测试\InfiniShell\infinishell.exe';
  OtherExe = 'C:\other\infinishell.exe';

procedure Require(Value: Boolean; const LabelText: String);
begin
  if not Value then
    RaiseException('FAIL: ' + LabelText);
  if not SaveStringToFile('{#EvidenceDir}\assertions.log', 'PASS: ' + LabelText + #13#10, True) then
    RaiseException('Cannot write assertion evidence');
end;

function Classes(const CaseName: String): String;
begin
  Result := TestRoot + '\' + CaseName;
end;

function Uri(const CaseName: String): String;
begin
  Result := Classes(CaseName) + '\infinishell';
end;

function Aumid(const CaseName: String): String;
begin
  Result := Classes(CaseName) + '\AppUserModelId\' + AppId;
end;

procedure RegisterCase(const CaseName: String);
begin
  Require(RegisterWindowsIntegration(HKCU, Classes(CaseName), 'infinishell', AppName, AppId, ExePath),
    CaseName + ': register');
end;

procedure RemoveCase(const CaseName: String);
begin
  Require(RemoveOwnedWindowsIntegration(HKCU, Classes(CaseName), 'infinishell', AppName, AppId, ExePath),
    CaseName + ': cleanup');
end;

function InitializeSetup(): Boolean;
begin
  { 此事件结束后返回 False；不进入安装、卸载、快捷方式或产品启动。 }
  Result := False;
  Require(RegistrationStringMatches(HKCU, TestRoot, 'TestId', '{#TestId}'), 'unique HKCU root');

  RegisterCase('fresh');
  Require(RegistrationStringMatches(HKCU, Uri('fresh') + '\shell\open\command', '',
    '"' + ExePath + '" "%1"'), 'fresh: quoted Unicode command');
  Require(RegistrationStringMatches(HKCU, Uri('fresh'), RegistrationOwnerValue, ExePath), 'fresh: URI owner');
  Require(RegistrationStringMatches(HKCU, Aumid('fresh'), RegistrationOwnerValue, ExePath), 'fresh: AUMID owner');
  RemoveCase('fresh');
  Require(not RegKeyExists(HKCU, Uri('fresh')), 'fresh: URI removed');
  Require(not RegKeyExists(HKCU, Aumid('fresh')), 'fresh: AUMID removed');

  RegisterCase('path-case');
  Require(RemoveOwnedWindowsIntegration(HKCU, Classes('path-case'), 'infinishell', AppName, AppId, Uppercase(ExePath)),
    'path-case: cleanup accepts Windows path casing');
  Require(not RegKeyExists(HKCU, Uri('path-case')), 'path-case: URI removed');
  Require(not RegKeyExists(HKCU, Aumid('path-case')), 'path-case: AUMID removed');

  Require(RegWriteStringValue(HKCU, Uri('stale') + '\shell\open\command', '',
    '"C:\missing\infinishell.exe" "%0"'), 'stale: seed legacy command');
  RegisterCase('stale');
  Require(RegistrationStringMatches(HKCU, Uri('stale') + '\shell\open\command', '',
    '"' + ExePath + '" "%1"'), 'stale: legacy target repaired');

  RegisterCase('foreign-command');
  Require(RegWriteStringValue(HKCU, Uri('foreign-command') + '\shell\open\command', '',
    '"' + OtherExe + '" "%0"'), 'foreign-command: seed old-version takeover');
  RemoveCase('foreign-command');
  Require(RegistrationStringMatches(HKCU, Uri('foreign-command') + '\shell\open\command', '',
    '"' + OtherExe + '" "%0"'), 'foreign-command: URI retained');
  Require(RegistrationStringMatches(HKCU, Aumid('foreign-command'), RegistrationOwnerValue, ExePath),
    'foreign-command: AUMID retained despite old owner');

  RegisterCase('foreign-owner');
  Require(RegWriteStringValue(HKCU, Uri('foreign-owner'), RegistrationOwnerValue, OtherExe), 'foreign-owner: seed');
  RemoveCase('foreign-owner');
  Require(RegistrationStringMatches(HKCU, Uri('foreign-owner'), RegistrationOwnerValue, OtherExe), 'foreign-owner: retained');
  Require(RegKeyExists(HKCU, Aumid('foreign-owner')), 'foreign-owner: AUMID retained');

  RegisterCase('no-owner');
  Require(RegDeleteValue(HKCU, Uri('no-owner'), RegistrationOwnerValue), 'no-owner: seed');
  RemoveCase('no-owner');
  Require(RegKeyExists(HKCU, Uri('no-owner') + '\shell\open\command'), 'no-owner: URI retained');
  Require(RegKeyExists(HKCU, Aumid('no-owner')), 'no-owner: AUMID retained');

  RegisterCase('wrong-type');
  Require(RegWriteDWordValue(HKCU, Uri('wrong-type'), RegistrationOwnerValue, 1), 'wrong-type: seed');
  RemoveCase('wrong-type');
  Require(RegKeyExists(HKCU, Uri('wrong-type') + '\shell\open\command'), 'wrong-type: URI retained');
  Require(RegKeyExists(HKCU, Aumid('wrong-type')), 'wrong-type: AUMID retained');

  RegisterCase('expand-owner');
  Require(RegWriteExpandStringValue(HKCU, Uri('expand-owner'), RegistrationOwnerValue, ExePath), 'expand-owner: seed');
  RemoveCase('expand-owner');
  Require(RegValueExists(HKCU, Uri('expand-owner'), RegistrationOwnerValue), 'expand-owner: URI retained');
  Require(RegKeyExists(HKCU, Aumid('expand-owner')), 'expand-owner: AUMID retained');

  RegisterCase('expand-command');
  Require(RegWriteExpandStringValue(HKCU, Uri('expand-command') + '\shell\open\command', '',
    '"' + ExePath + '" "%1"'), 'expand-command: seed');
  RemoveCase('expand-command');
  Require(RegValueExists(HKCU, Uri('expand-command') + '\shell\open\command', ''), 'expand-command: URI retained');
  Require(RegKeyExists(HKCU, Aumid('expand-command')), 'expand-command: AUMID retained');

  RegisterCase('expand-aumid');
  Require(RegWriteExpandStringValue(HKCU, Aumid('expand-aumid'), RegistrationOwnerValue, ExePath), 'expand-aumid: seed');
  RemoveCase('expand-aumid');
  Require(RegValueExists(HKCU, Aumid('expand-aumid'), RegistrationOwnerValue), 'expand-aumid: owner retained');
  Require(RegValueExists(HKCU, Aumid('expand-aumid'), 'DisplayName'), 'expand-aumid: display retained');

  RegisterCase('expand-display');
  Require(RegWriteExpandStringValue(HKCU, Aumid('expand-display'), 'DisplayName', AppName), 'expand-display: seed');
  RemoveCase('expand-display');
  Require(RegValueExists(HKCU, Aumid('expand-display'), RegistrationOwnerValue), 'expand-display: owner retained');
  Require(RegValueExists(HKCU, Aumid('expand-display'), 'DisplayName'), 'expand-display: display retained');

  RegisterCase('foreign-aumid');
  Require(RegWriteStringValue(HKCU, Aumid('foreign-aumid'), RegistrationOwnerValue, OtherExe), 'foreign-aumid: seed');
  RemoveCase('foreign-aumid');
  Require(RegistrationStringMatches(HKCU, Aumid('foreign-aumid'), RegistrationOwnerValue, OtherExe), 'foreign-aumid: retained');
  Require(not RegKeyExists(HKCU, Uri('foreign-aumid')), 'foreign-aumid: own URI removed');

  RegisterCase('foreign-display');
  Require(RegWriteStringValue(HKCU, Aumid('foreign-display'), 'DisplayName', 'Other app'), 'foreign-display: seed');
  RemoveCase('foreign-display');
  Require(RegistrationStringMatches(HKCU, Aumid('foreign-display'), 'DisplayName', 'Other app'), 'foreign-display: retained');

  RegisterCase('extra-values');
  Require(RegWriteStringValue(HKCU, Uri('extra-values') + '\shell\open\command', 'OtherValue', 'keep'), 'extra-values: seed command value');
  Require(RegWriteStringValue(HKCU, Uri('extra-values') + '\foreign', '', 'keep'), 'extra-values: seed URI subkey');
  Require(RegWriteStringValue(HKCU, Aumid('extra-values'), 'OtherValue', 'keep'), 'extra-values: seed AUMID value');
  RemoveCase('extra-values');
  Require(RegistrationStringMatches(HKCU, Uri('extra-values') + '\shell\open\command', 'OtherValue', 'keep'), 'extra-values: command value retained');
  Require(RegistrationStringMatches(HKCU, Uri('extra-values') + '\foreign', '', 'keep'), 'extra-values: URI subkey retained');
  Require(RegistrationStringMatches(HKCU, Aumid('extra-values'), 'OtherValue', 'keep'), 'extra-values: AUMID value retained');
  Require(not RegValueExists(HKCU, Aumid('extra-values'), RegistrationOwnerValue), 'extra-values: own AUMID marker removed');

  { PowerShell 仅在本次随机子树中设置拒绝 ACL，验证真实 Win32 失败传播。 }
  Require(not RegisterWindowsIntegration(HKCU, Classes('write-denied'), 'infinishell', AppName, AppId, ExePath),
    'write-denied: registration fails');
  RemoveCase('read-denied');
  Require(RegKeyExists(HKCU, Uri('read-denied') + '\shell\open\command'), 'read-denied: URI retained');
  Require(RegKeyExists(HKCU, Aumid('read-denied')), 'read-denied: AUMID retained');
  Require(not RemoveOwnedWindowsIntegration(HKCU, Classes('delete-denied'), 'infinishell', AppName, AppId, ExePath),
    'delete-denied: deletion failure propagated');
  Require(RegKeyExists(HKCU, Uri('delete-denied') + '\shell\open\command'), 'delete-denied: URI retained after failure');

  Require(SaveStringToFile('{#EvidenceDir}\completed.txt', '{#TestId}', False), 'completion receipt');
end;
