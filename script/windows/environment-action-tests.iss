[Setup]
AppId=InfiniShellEnvironmentAction-{#TestId}-{#HelperFlavor}
AppName=InfiniShell Environment Action
AppVersion=1
DefaultDirName={tmp}\unused
CreateAppDir=no
Uninstallable=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputBaseFilename=environment-action-{#HelperFlavor}
OutputDir={#EvidenceDir}
Compression=none

#include HelperFile

[Code]
function InitializeSetup(): Boolean;
var
  Identity, Name, Action, Product, Path, Key, Receipt: String;
  Success: Boolean;
begin
  { 独立进程只调用选定版本的真实 helper，不进入产品安装。 }
  Result := False;
  Key := 'Software\InfiniShellEnvironmentTests\{#TestId}';
  if not RegQueryStringValue(HKCU, Key, 'TestId', Identity) or (Identity <> '{#TestId}') then
    RaiseException('Isolated registry identity mismatch');
  Name := ExpandConstant('{param:case|}');
  Action := ExpandConstant('{param:action|}');
  Product := ExpandConstant('{param:product|}');
  Receipt := ExpandConstant('{param:receipt|}');
  if (Name = '') or (Pos('\', Name) > 0) or (Receipt = '') or (Pos('\', Receipt) > 0) then
    RaiseException('Invalid isolated action arguments');
  if Product = 'A' then Path := 'C:\安装 测试\InfiniShell\bin'
  else if Product = 'B' then Path := 'D:\TUI Tools\bin'
  else if Product = 'C' then Path := 'E:\第三个 产品\bin'
  else RaiseException('Invalid product');
  Key := Key + '\' + Name;
  if Action = 'add' then Success := AddEnvironmentPath(HKCU, Key, Path)
  else if Action = 'remove' then Success := RemoveEnvironmentPath(HKCU, Key, Path)
  else RaiseException('Invalid action');
  if not SaveStringToFile('{#EvidenceDir}\' + Receipt + '.txt',
    '{#TestId}:' + IntToStr(Ord(Success)), False) then RaiseException('Cannot write action receipt');
end;
