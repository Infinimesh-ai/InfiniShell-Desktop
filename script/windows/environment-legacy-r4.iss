[Code]

const
  SystemEnvironmentKey = 'SYSTEM\CurrentControlSet\Control\Session Manager\Environment';
  UserEnvironmentKey = 'Environment';

function EnvironmentOpenKey(RootKey: Integer; Subkey: String; Options, Access: LongWord;
  var OpenedKey: THandle): Longint;
  external 'RegOpenKeyExW@advapi32.dll stdcall';
function EnvironmentQueryType(Key: THandle; Name: String; Reserved: DWORD_PTR;
  var ValueType: LongWord; Data: DWORD_PTR; var DataSize: LongWord): Longint;
  external 'RegQueryValueExW@advapi32.dll stdcall';
function EnvironmentSetString(Key: THandle; Name: String; Reserved, ValueType: LongWord;
  Data: String; DataSize: LongWord): Longint;
  external 'RegSetValueExW@advapi32.dll stdcall';
function EnvironmentCloseKey(Key: THandle): Longint;
  external 'RegCloseKey@advapi32.dll stdcall';
function EnvironmentExpandString(Source, Buffer: String; Size: LongWord): LongWord;
  external 'ExpandEnvironmentStringsW@kernel32.dll stdcall';

procedure GetEnvironmentKeyInfo(var RootKey: Integer; var SubKey: string);
begin
  if IsAdminInstallMode then begin
    RootKey := HKEY_LOCAL_MACHINE;
    SubKey := SystemEnvironmentKey;
  end else begin
    RootKey := HKEY_CURRENT_USER;
    SubKey := UserEnvironmentKey;
  end;
end;

function EnvironmentAccess(Access: LongWord): LongWord;
begin
  if Is64BitInstallMode then Result := Access or $0100
  else Result := Access or $0200;
end;

function ReadEnvironmentPath(RootKey: Integer; const SubKey: String;
  var Value: String; var ValueType: LongWord): Boolean;
var
  Key: THandle;
  Status: Longint;
  Size: LongWord;
begin
  { 类型 0 专指不存在；读取失败和非字符串类型不能当作空 PATH。 }
  Result := False;
  Value := '';
  ValueType := 0;
  Status := EnvironmentOpenKey(RootKey, SubKey, 0, EnvironmentAccess($0001), Key);
  if (Status = 2) or (Status = 3) then begin Result := True; exit; end;
  if Status <> 0 then exit;
  try
    Size := 0;
    Status := EnvironmentQueryType(Key, 'Path', 0, ValueType, 0, Size);
    if Status = 2 then begin ValueType := 0; Result := True; exit; end;
    if (Status <> 0) or ((ValueType <> 1) and (ValueType <> 2)) then exit;
    { Inno 的读取保留 REG_EXPAND_SZ 原文，不展开其中的环境变量。 }
    Result := RegQueryStringValue(RootKey, SubKey, 'Path', Value);
  finally
    EnvironmentCloseKey(Key);
  end;
end;

function WriteEnvironmentPath(RootKey: Integer; const SubKey, Value: String;
  ValueType: LongWord): Boolean;
var
  Key: THandle;
  Actual: String;
  ActualType: LongWord;
begin
  Result := False;
  if ValueType = 0 then Result := RegDeleteValue(RootKey, SubKey, 'Path')
  else begin
    if EnvironmentOpenKey(RootKey, SubKey, 0, EnvironmentAccess($0002), Key) <> 0 then exit;
    try
      { 显式写入原类型；显式终止符还避免空字符串被外部调用封送成空指针。 }
      Result := EnvironmentSetString(Key, 'Path', 0, ValueType, Value + #0,
        (Length(Value) + 1) * 2) = 0;
    finally
      EnvironmentCloseKey(Key);
    end;
  end;
  if Result then Result := ReadEnvironmentPath(RootKey, SubKey, Actual, ActualType);
  if Result then Result := (ActualType = ValueType) and (Actual = Value);
end;

function EnvironmentPathIdentity(const Path: String): String;
begin
  Result := Trim(RemoveQuotes(Trim(Path)));
  StringChangeEx(Result, '/', '\', True);
  Result := Uppercase(RemoveBackslashUnlessRoot(Result));
end;

function EnvironmentPathCount(const Paths, Path: String; ValueType: LongWord;
  var Position: Integer): Integer;
var
  Start, Finish: Integer;
  Token, Expanded: String;
  Size, Written: LongWord;
begin
  Result := 0;
  Position := 0;
  Start := 1;
  for Finish := 1 to Length(Paths) + 1 do
    if (Finish > Length(Paths)) or (Copy(Paths, Finish, 1) = ';') then begin
      Token := Copy(Paths, Start, Finish - Start);
      { 仅比较 REG_EXPAND_SZ 时展开变量，写回始终使用原文。 }
      if ValueType = 2 then begin
        Size := EnvironmentExpandString(Token + #0, '', 0);
        if Size > 0 then begin
          Expanded := StringOfChar(#0, Size);
          Written := EnvironmentExpandString(Token + #0, Expanded, Size);
          if (Written > 0) and (Written <= Size) then Token := Copy(Expanded, 1, Written - 1);
        end;
      end;
      if EnvironmentPathIdentity(Token) = EnvironmentPathIdentity(Path) then begin
        Result := Result + 1;
        Position := Start;
      end;
      Start := Finish + 1;
    end;
end;

function EnvironmentOwnershipKey(const SubKey, Path: String): String;
begin
  { 与产品卸载键分开保存；同目录升级继续使用首次添加时的记录。 }
  Result := SubKey + '\InfiniShellPathOwnership\' +
    GetSHA256OfUnicodeString(EnvironmentPathIdentity(Path));
end;

function AppendedEnvironmentPath(const Original, Path: String): String;
begin
  Result := Original;
  if Result <> '' then Result := Result + ';';
  Result := Result + Path;
end;

function ForgetEnvironmentOwnership(RootKey: Integer; const SubKey, Key: String): Boolean;
begin
  Result := False;
  if RegValueExists(RootKey, Key, 'Committed') then
    if not RegDeleteValue(RootKey, Key, 'Committed') then exit;
  if RegValueExists(RootKey, Key, 'OriginalType') then
    if not RegDeleteValue(RootKey, Key, 'OriginalType') then exit;
  if RegValueExists(RootKey, Key, 'OriginalValue') then
    if not RegDeleteValue(RootKey, Key, 'OriginalValue') then exit;
  if RegValueExists(RootKey, Key, 'AddedPath') then
    if not RegDeleteValue(RootKey, Key, 'AddedPath') then exit;
  RegDeleteKeyIfEmpty(RootKey, Key);
  RegDeleteKeyIfEmpty(RootKey, SubKey + '\InfiniShellPathOwnership');
  Result := True;
end;

function AddEnvironmentPath(RootKey: Integer; const SubKey, Path: String): Boolean;
var
  Original, Installed, Current, Key: String;
  OriginalType, InstalledType, CurrentType: LongWord;
  Position: Integer;
begin
  Result := False;
  if (EnvironmentPathIdentity(Path) = '') or (Pos(';', Path) > 0) then exit;
  if not ReadEnvironmentPath(RootKey, SubKey, Original, OriginalType) then exit;
  { 已存在项不归本次安装所有；升级也不能覆盖已有的首次备份。 }
  if EnvironmentPathCount(Original, Path, OriginalType, Position) > 0 then begin Result := True; exit; end;
  Key := EnvironmentOwnershipKey(SubKey, Path);
  Installed := AppendedEnvironmentPath(Original, Path);
  InstalledType := OriginalType;
  if InstalledType = 0 then InstalledType := 2;
  if not RegWriteDWordValue(RootKey, Key, 'Committed', 0) then exit;
  if not RegWriteDWordValue(RootKey, Key, 'OriginalType', OriginalType) or
    not RegWriteStringValue(RootKey, Key, 'OriginalValue', Original) or
    not RegWriteStringValue(RootKey, Key, 'AddedPath', Path) then begin
    ForgetEnvironmentOwnership(RootKey, SubKey, Key);
    exit;
  end;
  { 在提交 PATH 前再次核对，避免覆盖准备记录期间的其它修改。 }
  if ReadEnvironmentPath(RootKey, SubKey, Current, CurrentType) and
    (CurrentType = OriginalType) and (Current = Original) then
    Result := WriteEnvironmentPath(RootKey, SubKey, Installed, InstalledType);
  if Result then Result := RegWriteDWordValue(RootKey, Key, 'Committed', 1);
  if not Result then begin
    { 只有实际仍是刚写入的值才回滚；失败记录不构成所有权。 }
    if ReadEnvironmentPath(RootKey, SubKey, Current, CurrentType) and
      (CurrentType = InstalledType) and (Current = Installed) then
      WriteEnvironmentPath(RootKey, SubKey, Original, OriginalType);
    ForgetEnvironmentOwnership(RootKey, SubKey, Key);
  end;
end;

function RemoveEnvironmentPath(RootKey: Integer; const SubKey, Path: String): Boolean;
var
  Key, Original, AddedPath, Current, Installed, Updated: String;
  Committed, OriginalType, InstalledType, CurrentType: LongWord;
  Position, Count, DeleteAt, DeleteLength: Integer;
begin
  Result := True;
  Key := EnvironmentOwnershipKey(SubKey, Path);
  { 旧版本和安装前已有项没有可验证归属，保守保留。 }
  if not RegKeyExists(RootKey, Key) then exit;
  Result := False;
  if not RegQueryDWordValue(RootKey, Key, 'Committed', Committed) or (Committed <> 1) or
    not RegQueryDWordValue(RootKey, Key, 'OriginalType', OriginalType) or (OriginalType > 2) or
    not RegQueryStringValue(RootKey, Key, 'OriginalValue', Original) or
    not RegQueryStringValue(RootKey, Key, 'AddedPath', AddedPath) then exit;
  if EnvironmentPathIdentity(AddedPath) <> EnvironmentPathIdentity(Path) then exit;
  if not ReadEnvironmentPath(RootKey, SubKey, Current, CurrentType) then exit;
  { 先撤销提交标记，避免 PATH 已移除但记录清理失败后仍声称拥有该项。 }
  if not RegWriteDWordValue(RootKey, Key, 'Committed', 0) then exit;
  Installed := AppendedEnvironmentPath(Original, AddedPath);
  InstalledType := OriginalType;
  if InstalledType = 0 then InstalledType := 2;
  if (Current = Installed) and (CurrentType = InstalledType) then
    Result := WriteEnvironmentPath(RootKey, SubKey, Original, OriginalType)
  else begin
    Count := EnvironmentPathCount(Current, AddedPath, CurrentType, Position);
    Result := True;
    if Count = 1 then begin
      { 用户改过其它内容时，仅删除原文仍匹配的唯一新增项，保留当前类型。 }
      if (Copy(Current, Position, Length(AddedPath)) = AddedPath) and
        ((Position + Length(AddedPath) > Length(Current)) or
         (Copy(Current, Position + Length(AddedPath), 1) = ';')) then begin
        DeleteAt := Position;
        DeleteLength := Length(AddedPath);
        if Position > 1 then begin
          DeleteAt := Position - 1;
          DeleteLength := DeleteLength + 1;
        end else if Length(Current) > DeleteLength then DeleteLength := DeleteLength + 1;
        Updated := Current;
        Delete(Updated, DeleteAt, DeleteLength);
        Result := WriteEnvironmentPath(RootKey, SubKey, Updated, CurrentType);
      end else Log('PATH 项原文已变化，保留并放弃旧归属。');
    end else if Count > 1 then Log('PATH 存在多个等价项，无法确认归属，保留用户修改。');
  end;
  if Result then Result := ForgetEnvironmentOwnership(RootKey, SubKey, Key)
  else RegWriteDWordValue(RootKey, Key, 'Committed', 1);
end;

procedure EnvAddPath(Path: string);
var
  RootKey: Integer;
  SubKey: String;
begin
  GetEnvironmentKeyInfo(RootKey, SubKey);
  if not AddEnvironmentPath(RootKey, SubKey, Path) then
    RaiseException(FmtMessage(SetupMessage(msgErrorFunctionFailedNoCode), ['EnvAddPath']));
end;

procedure EnvRemovePath(Path: string);
var
  RootKey: Integer;
  SubKey: String;
begin
  GetEnvironmentKeyInfo(RootKey, SubKey);
  if not RemoveEnvironmentPath(RootKey, SubKey, Path) then
    RaiseException(FmtMessage(SetupMessage(msgErrorFunctionFailedNoCode), ['EnvRemovePath']));
end;
