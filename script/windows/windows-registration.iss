[Code]
const
  RegistrationOwnerValue = 'InfiniShellOwnerPath';

{ Inno 6.7 的注册表根类型为 Integer，Win32 句柄为 THandle；
  不使用较新版帮助页面中的 HKEY/NativeUInt。 }
function RegistrationOpenKey(RootKey: Integer; Subkey: String; Options, Access: LongWord;
  var OpenedKey: THandle): Longint;
  external 'RegOpenKeyExW@advapi32.dll stdcall';
function RegistrationQueryType(Key: THandle; Name: String; Reserved: DWORD_PTR;
  var ValueType: LongWord; Data: DWORD_PTR; var DataSize: LongWord): Longint;
  external 'RegQueryValueExW@advapi32.dll stdcall';
function RegistrationCloseKey(Key: THandle): Longint;
  external 'RegCloseKey@advapi32.dll stdcall';

function IsRegistrationString(RootKey: Integer; const Key, Name: String): Boolean;
var
  OpenedKey: THandle;
  Access, ValueType, DataSize: LongWord;
begin
  { Inno 的字符串读取还接受 REG_EXPAND_SZ；归属协议只接受实际 REG_SZ。 }
  Result := False;
  Access := $0001; { KEY_QUERY_VALUE }
  if Is64BitInstallMode then
    Access := Access or $0100 { KEY_WOW64_64KEY }
  else
    Access := Access or $0200; { KEY_WOW64_32KEY，与 Inno 默认注册表视图一致。 }
  if RegistrationOpenKey(RootKey, Key, 0, Access, OpenedKey) <> 0 then exit;
  try
    DataSize := 0;
    if RegistrationQueryType(OpenedKey, Name, 0, ValueType, 0, DataSize) = 0 then
      Result := ValueType = 1; { REG_SZ }
  finally
    RegistrationCloseKey(OpenedKey);
  end;
end;

function RegistrationStringMatches(RootKey: Integer; const Key, Name, Expected: String): Boolean;
var
  Value: String;
begin
  { 读取失败或类型不符都不能当作不存在，更不能据此删除。 }
  Result := IsRegistrationString(RootKey, Key, Name);
  if Result then
    Result := RegQueryStringValue(RootKey, Key, Name, Value);
  if Result then
    Result := Value = Expected;
  if not Result then
    Log('注册表值未确认匹配，保留已有登记：' + Key + '\' + Name);
end;

function WriteRegistrationString(RootKey: Integer; const Key, Name, Value: String): Boolean;
begin
  Result := RegWriteStringValue(RootKey, Key, Name, Value);
  if Result then
    Result := RegistrationStringMatches(RootKey, Key, Name, Value);
  if not Result then
    Log('注册表写入或读回失败：' + Key + '\' + Name);
end;

function RegistrationPathMatches(RootKey: Integer; const Key, Name, Expected: String): Boolean;
var
  Value: String;
begin
  { 仅路径以及参数固定为 "%1" 的命令允许 Windows 路径大小写差异。 }
  Result := IsRegistrationString(RootKey, Key, Name);
  if Result then
    Result := RegQueryStringValue(RootKey, Key, Name, Value);
  if Result then
    Result := CompareText(Value, Expected) = 0;
  if not Result then
    Log('注册表路径未确认归属，保留已有登记：' + Key + '\' + Name);
end;

function RegisterWindowsIntegration(RootKey: Integer;
  const ClassesKey, Scheme, AppName, AppId, ExePath: String): Boolean;
var
  UriKey, AumidKey, DisplayName: String;
begin
  Result := False;
  UriKey := ClassesKey + '\' + Scheme;
  AumidKey := ClassesKey + '\AppUserModelId\' + AppId;
  DisplayName := Copy(AppId, RPos('.', AppId) + 1, Length(AppId));
  if not WriteRegistrationString(RootKey, UriKey, '', AppName) then exit;
  if not WriteRegistrationString(RootKey, UriKey, 'URL Protocol', '') then exit;
  if not WriteRegistrationString(RootKey, UriKey + '\shell\open\command', '',
    '"' + ExePath + '" "%1"') then exit;
  { 与运行时共用归属字段，只有命令已成功写入并读回才登记所有者。 }
  if not WriteRegistrationString(RootKey, UriKey, RegistrationOwnerValue, ExePath) then exit;
  if not WriteRegistrationString(RootKey, AumidKey, 'DisplayName', DisplayName) then exit;
  if not WriteRegistrationString(RootKey, AumidKey, RegistrationOwnerValue, ExePath) then exit;
  Result := True;
end;

function DeleteRegistrationValue(RootKey: Integer; const Key, Name: String): Boolean;
begin
  Result := RegDeleteValue(RootKey, Key, Name);
  if not Result then
    Log('删除已确认归属的注册表值失败：' + Key + '\' + Name);
end;

function PruneRegistrationKey(RootKey: Integer; const Key: String): Boolean;
var
  Values, Subkeys: TArrayOfString;
begin
  Result := False;
  if not RegGetValueNames(RootKey, Key, Values) then
  begin
    Log('读取注册表值列表失败，保留该键：' + Key);
    exit;
  end;
  if not RegGetSubkeyNames(RootKey, Key, Subkeys) then
  begin
    Log('读取注册表子键列表失败，保留该键：' + Key);
    exit;
  end;
  { 附加值和子键可能来自其他应用，只收缩完全为空的键。 }
  if (GetArrayLength(Values) > 0) or (GetArrayLength(Subkeys) > 0) then
    Result := True
  else
    Result := RegDeleteKeyIfEmpty(RootKey, Key);
  if not Result then
    Log('检查或删除空注册表键失败：' + Key);
end;

function RemoveOwnedWindowsIntegration(RootKey: Integer;
  const ClassesKey, Scheme, AppName, AppId, ExePath: String): Boolean;
var
  UriKey, CommandKey, AumidKey, DisplayName: String;
begin
  Result := True;
  UriKey := ClassesKey + '\' + Scheme;
  CommandKey := UriKey + '\shell\open\command';
  AumidKey := ClassesKey + '\AppUserModelId\' + AppId;
  DisplayName := Copy(AppId, RPos('.', AppId) + 1, Length(AppId));
  { 先确定 URI 的当前归属。旧版本可能改写命令却留下本版本的 owner；
    这种接管同样必须保留 AUMID，不能只凭历史 marker 删除。 }
  if not RegistrationPathMatches(RootKey, UriKey, RegistrationOwnerValue, ExePath) then exit;
  if not RegistrationPathMatches(RootKey, CommandKey, '', '"' + ExePath + '" "%1"') then exit;

  if RegistrationPathMatches(RootKey, AumidKey, RegistrationOwnerValue, ExePath) and
    RegistrationStringMatches(RootKey, AumidKey, 'DisplayName', DisplayName) then
  begin
    Result := False;
    if not DeleteRegistrationValue(RootKey, AumidKey, 'DisplayName') then exit;
    if not DeleteRegistrationValue(RootKey, AumidKey, RegistrationOwnerValue) then exit;
    if not PruneRegistrationKey(RootKey, AumidKey) then exit;
  end;

  Result := False;
  if not DeleteRegistrationValue(RootKey, CommandKey, '') then exit;
  if not PruneRegistrationKey(RootKey, CommandKey) then exit;
  if not PruneRegistrationKey(RootKey, UriKey + '\shell\open') then exit;
  if not PruneRegistrationKey(RootKey, UriKey + '\shell') then exit;
  if RegistrationStringMatches(RootKey, UriKey, '', AppName) then
    if not DeleteRegistrationValue(RootKey, UriKey, '') then exit;
  if RegistrationStringMatches(RootKey, UriKey, 'URL Protocol', '') then
    if not DeleteRegistrationValue(RootKey, UriKey, 'URL Protocol') then exit;
  if not DeleteRegistrationValue(RootKey, UriKey, RegistrationOwnerValue) then exit;
  Result := PruneRegistrationKey(RootKey, UriKey);
end;
