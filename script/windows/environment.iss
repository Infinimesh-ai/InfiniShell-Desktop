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
function EnvironmentQueryString(Key: THandle; Name: String; Reserved: DWORD_PTR;
  var ValueType: LongWord; Data: String; var DataSize: LongWord): Longint;
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

function ReadEnvironmentValue(RootKey: Integer; const SubKey, Name: String;
  var Value: String; var ValueType: LongWord): Boolean;
var
  Key: THandle;
  Status: Longint;
  Size: LongWord;
  Buffer: String;
  I, UnitValue: Integer;
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
    Status := EnvironmentQueryType(Key, Name, 0, ValueType, 0, Size);
    if Status = 2 then begin ValueType := 0; Result := True; exit; end;
    if (Status <> 0) or ((ValueType <> 1) and (ValueType <> 2)) then exit;
    { 直接读取原始 UTF-16；Inno 的字符串读取会丢弃尾部 NUL，不能用于字节保真判断。 }
    if (Size < 2) or (Size mod 2 <> 0) then exit;
    Buffer := StringOfChar(#0, Size div 2);
    Status := EnvironmentQueryString(Key, Name, 0, ValueType, Buffer, Size);
    if (Status <> 0) or ((ValueType <> 1) and (ValueType <> 2)) or
      (Size < 2) or (Size mod 2 <> 0) or (Size div 2 > Length(Buffer)) then exit;
    if Buffer[Size div 2] <> #0 then exit;
    Buffer := Copy(Buffer, 1, Size div 2 - 1);
    { 只接受一个末尾 NUL 和完整代理对；异常值在归属记录或 PATH 写入前拒绝。 }
    I := 1;
    while I <= Length(Buffer) do begin
      UnitValue := Ord(Buffer[I]);
      if (UnitValue = 0) or ((UnitValue >= $DC00) and (UnitValue <= $DFFF)) then exit;
      if (UnitValue >= $D800) and (UnitValue <= $DBFF) then begin
        I := I + 1;
        if I > Length(Buffer) then exit;
        UnitValue := Ord(Buffer[I]);
        if (UnitValue < $DC00) or (UnitValue > $DFFF) then exit;
      end;
      I := I + 1;
    end;
    Value := Buffer;
    Result := True;
  finally
    EnvironmentCloseKey(Key);
  end;
end;

function ReadEnvironmentPath(RootKey: Integer; const SubKey: String;
  var Value: String; var ValueType: LongWord): Boolean;
begin
  Result := ReadEnvironmentValue(RootKey, SubKey, 'Path', Value, ValueType);
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
  { 空子键也是归属链的一部分；删除被拒绝时保留意图供重试。 }
  if RegKeyExists(RootKey, Key) then
    if not RegDeleteKeyIfEmpty(RootKey, Key) then exit;
  Result := True;
end;

function EnvironmentState(const Value: String; ValueType: LongWord): String;
begin
  Result := IntToStr(ValueType) + Value;
end;

function ValidEnvironmentState(const State: String): Boolean;
begin
  Result := (State <> '') and (Pos(Copy(State, 1, 1), '012') > 0) and
    ((Copy(State, 1, 1) <> '0') or (Length(State) = 1));
end;

function ReadEnvironmentState(RootKey: Integer; const SubKey: String;
  var State: String): Boolean;
var
  Value: String;
  ValueType: LongWord;
begin
  Result := ReadEnvironmentPath(RootKey, SubKey, Value, ValueType);
  State := EnvironmentState(Value, ValueType);
end;

function WriteEnvironmentState(RootKey: Integer; const SubKey, State: String): Boolean;
begin
  Result := WriteEnvironmentPath(RootKey, SubKey, Copy(State, 2, MaxInt),
    StrToInt(Copy(State, 1, 1)));
end;

function EnvironmentField(const Value: String): String;
begin
  Result := IntToStr(Length(Value)) + ':' + Value;
end;

function TakeEnvironmentField(var Data: String; var Value: String): Boolean;
var
  Split, Size: Integer;
begin
  Result := False;
  Split := Pos(':', Data);
  if Split = 0 then exit;
  Size := StrToIntDef(Copy(Data, 1, Split - 1), -1);
  if (Size < 0) or (Size > Length(Data) - Split) then exit;
  Value := Copy(Data, Split + 1, Size);
  Delete(Data, 1, Split + Size);
  Result := True;
end;

function EnvironmentGroupKey(const SubKey: String): String;
begin
  Result := SubKey + '\InfiniShellPathOwnership';
end;

function EnvironmentGroupFields(RootKey: Integer; const SubKey: String): Boolean;
var
  Names: TArrayOfString;
  I: Integer;
begin
  Result := True;
  if not RegKeyExists(RootKey, EnvironmentGroupKey(SubKey)) then exit;
  Result := False;
  if not RegGetValueNames(RootKey, EnvironmentGroupKey(SubKey), Names) then exit;
  for I := 0 to GetArrayLength(Names) - 1 do
    if (CompareText(Names[I], 'State') <> 0) and (CompareText(Names[I], 'Pending') <> 0) and
      (CompareText(Names[I], 'Projection') <> 0) then exit;
  Result := True;
end;

function SaveEnvironmentGroup(RootKey: Integer; const SubKey, Base, Known: String;
  Trusted: Boolean): Boolean;
var
  Flag: String;
begin
  Flag := '0';
  if Trusted then Flag := '1';
  { 单个值提交完整状态，防止类型、原文和可信标志只写入一部分。 }
  Result := RegWriteStringValue(RootKey, EnvironmentGroupKey(SubKey), 'State',
    '1' + Flag + EnvironmentField(Base) + EnvironmentField(Known));
end;

function LoadEnvironmentGroup(RootKey: Integer; const SubKey: String;
  var Base, Known: String; var Trusted, Exists: Boolean): Boolean;
var
  Data: String;
  ValueType: LongWord;
begin
  Result := False;
  Exists := False;
  if not ReadEnvironmentValue(RootKey, EnvironmentGroupKey(SubKey), 'State', Data, ValueType) then exit;
  if ValueType = 0 then begin Result := True; exit; end;
  Exists := True;
  if (ValueType <> 1) or (Copy(Data, 1, 1) <> '1') or
    (Pos(Copy(Data, 2, 1), '01') = 0) or (Length(Data) < 2) then exit;
  Trusted := Copy(Data, 2, 1) = '1';
  Delete(Data, 1, 2);
  Result := TakeEnvironmentField(Data, Base) and TakeEnvironmentField(Data, Known) and
    (Data = '') and ValidEnvironmentState(Base) and ValidEnvironmentState(Known);
end;

function EnvironmentOwnerFields(RootKey: Integer; const Key: String; Complete: Boolean): Boolean;
var
  Names, Children: TArrayOfString;
  I: Integer;
begin
  Result := False;
  if not RegGetValueNames(RootKey, Key, Names) or not RegGetSubkeyNames(RootKey, Key, Children) then exit;
  if (GetArrayLength(Children) <> 0) or (Complete and (GetArrayLength(Names) <> 4)) then exit;
  for I := 0 to GetArrayLength(Names) - 1 do
    if (CompareText(Names[I], 'Committed') <> 0) and (CompareText(Names[I], 'OriginalType') <> 0) and
      (CompareText(Names[I], 'OriginalValue') <> 0) and (CompareText(Names[I], 'AddedPath') <> 0) then exit;
  Result := True;
end;

function ReadEnvironmentOwner(RootKey: Integer; const Key: String;
  var Original, Added: String; var Committed: LongWord): Boolean;
var
  Value: String;
  ValueType, StorageType: LongWord;
begin
  Result := EnvironmentOwnerFields(RootKey, Key, True) and
    RegQueryDWordValue(RootKey, Key, 'Committed', Committed) and (Committed <= 1) and
    RegQueryDWordValue(RootKey, Key, 'OriginalType', ValueType) and (ValueType <= 2) and
    ReadEnvironmentValue(RootKey, Key, 'OriginalValue', Value, StorageType) and (StorageType = 1) and
    ReadEnvironmentValue(RootKey, Key, 'AddedPath', Added, StorageType) and (StorageType = 1);
  if Result then begin
    Original := EnvironmentState(Value, ValueType);
    Result := ValidEnvironmentState(Original) and (EnvironmentPathIdentity(Added) <> '') and
      (Pos(';', Added) = 0) and
      (CompareText(ExtractFileName(Key), GetSHA256OfUnicodeString(EnvironmentPathIdentity(Added))) = 0);
  end;
end;

function AppendedEnvironmentState(const Original, Path: String): String;
var
  ValueType: LongWord;
begin
  ValueType := StrToInt(Copy(Original, 1, 1));
  if ValueType = 0 then ValueType := 2;
  Result := EnvironmentState(AppendedEnvironmentPath(Copy(Original, 2, MaxInt), Path), ValueType);
end;

function EnvironmentOwnerNames(RootKey: Integer; const SubKey: String;
  var Names: TArrayOfString): Boolean;
begin
  SetArrayLength(Names, 0);
  Result := True;
  if RegKeyExists(RootKey, EnvironmentGroupKey(SubKey)) then
    Result := RegGetSubkeyNames(RootKey, EnvironmentGroupKey(SubKey), Names);
end;

function EnvironmentSequenceMatches(RootKey: Integer; const SubKey, Base, Current: String;
  const Names: TArrayOfString): Boolean;
var
  Step, I, Position, Previous, NextPosition: Integer;
  Original, Added, NextPath, Expected, Value, Token: String;
  Committed, ValueType: LongWord;
begin
  Result := False;
  Value := Copy(Current, 2, MaxInt);
  ValueType := StrToInt(Copy(Current, 1, 1));
  Previous := 0;
  Expected := Base;
  for Step := 0 to GetArrayLength(Names) - 1 do begin
    NextPosition := MaxInt;
    for I := 0 to GetArrayLength(Names) - 1 do begin
      if not ReadEnvironmentOwner(RootKey, EnvironmentGroupKey(SubKey) + '\' + Names[I],
        Original, Added, Committed) or (Committed <> 1) then exit;
      if EnvironmentPathCount(Value, Added, ValueType, Position) <> 1 then exit;
      Token := Copy(Value, Position, Length(Added));
      if (Token <> Added) or
        ((Position + Length(Added) <= Length(Value)) and
         (Copy(Value, Position + Length(Added), 1) <> ';')) then exit;
      if (Position > Previous) and (Position < NextPosition) then begin
        NextPosition := Position;
        NextPath := Added;
      end;
    end;
    if NextPosition = MaxInt then exit;
    Expected := AppendedEnvironmentState(Expected, NextPath);
    Previous := NextPosition;
  end;
  Result := Expected = Current;
end;

function EnsureEnvironmentGroup(RootKey: Integer; const SubKey, Current: String;
  var Base, Known: String; var Trusted: Boolean): Boolean;
var
  Exists, Found, Ambiguous: Boolean;
  Names: TArrayOfString;
  I: Integer;
  Original, Added: String;
  Committed: LongWord;
begin
  Result := False;
  if not LoadEnvironmentGroup(RootKey, SubKey, Base, Known, Trusted, Exists) then exit;
  if not EnvironmentOwnerNames(RootKey, SubKey, Names) then exit;
  for I := 0 to GetArrayLength(Names) - 1 do
    if not ReadEnvironmentOwner(RootKey, EnvironmentGroupKey(SubKey) + '\' + Names[I],
      Original, Added, Committed) or (Committed <> 1) then exit;
  if not Exists then begin
    Base := Current;
    Found := GetArrayLength(Names) = 0;
    Ambiguous := False;
    { 存活记录提供候选锚；必须按实际顺序完整重建原文，不能只比较项目集合。 }
    for I := 0 to GetArrayLength(Names) - 1 do begin
      if not ReadEnvironmentOwner(RootKey, EnvironmentGroupKey(SubKey) + '\' + Names[I],
        Original, Added, Committed) then exit;
      if EnvironmentSequenceMatches(RootKey, SubKey, Original, Current, Names) then begin
        if Found and (Base <> Original) then Ambiguous := True;
        Base := Original;
        Found := True;
      end;
    end;
    Trusted := Found and not Ambiguous;
  end else begin
    { R5 已有失信标记或 Known 失配时，不能用旧锚重新取得不存在值的恢复权。 }
    Trusted := Trusted and (Known = Current) and
      EnvironmentSequenceMatches(RootKey, SubKey, Base, Current, Names);
  end;
  Known := Current;
  Result := SaveEnvironmentGroup(RootKey, SubKey, Base, Known, Trusted);
end;

function CleanEnvironmentGroup(RootKey: Integer; const SubKey: String): Boolean;
var
  Names: TArrayOfString;
begin
  Result := False;
  if not EnvironmentOwnerNames(RootKey, SubKey, Names) then exit;
  if GetArrayLength(Names) = 0 then begin
    if RegValueExists(RootKey, EnvironmentGroupKey(SubKey), 'State') then
      if not RegDeleteValue(RootKey, EnvironmentGroupKey(SubKey), 'State') then exit;
    if RegKeyExists(RootKey, EnvironmentGroupKey(SubKey)) then
      if not RegDeleteKeyIfEmpty(RootKey, EnvironmentGroupKey(SubKey)) then exit;
  end;
  Result := True;
end;

function SaveEnvironmentPending(RootKey: Integer; const SubKey, Operation, Path,
  BeforeState, AfterState: String): Boolean;
begin
  { 先保存完整操作意图，才允许修改 PATH；恢复只根据实际前后状态决定。 }
  Result := RegWriteStringValue(RootKey, EnvironmentGroupKey(SubKey), 'Pending',
    '1' + Operation + EnvironmentField(Path) + EnvironmentField(BeforeState) +
    EnvironmentField(AfterState));
end;

function RecoverEnvironmentProjection(RootKey: Integer; const SubKey: String): Boolean; forward;

function RecoverEnvironmentPending(RootKey: Integer; const SubKey: String): Boolean;
var
  Data, Operation, Path, BeforeState, AfterState, Current, Base, Known, Key, Original, Added: String;
  ValueType, Committed: LongWord;
  Trusted, Exists: Boolean;
begin
  Result := False;
  if not EnvironmentGroupFields(RootKey, SubKey) or
    not RecoverEnvironmentProjection(RootKey, SubKey) then exit;
  if not ReadEnvironmentValue(RootKey, EnvironmentGroupKey(SubKey), 'Pending', Data, ValueType) then exit;
  if ValueType = 0 then begin
    { 先验证状态，损坏的元数据原样保留；结束批次不能污染下一次安装。 }
    if not LoadEnvironmentGroup(RootKey, SubKey, Base, Known, Trusted, Exists) then exit;
    Result := CleanEnvironmentGroup(RootKey, SubKey);
    exit;
  end;
  Operation := Copy(Data, 2, 1);
  if (ValueType <> 1) or (Copy(Data, 1, 1) <> '1') or
    ((Operation <> 'A') and (Operation <> 'R')) then exit;
  Delete(Data, 1, 2);
  if not TakeEnvironmentField(Data, Path) or not TakeEnvironmentField(Data, BeforeState) or
    not TakeEnvironmentField(Data, AfterState) or (Data <> '') or
    not ValidEnvironmentState(BeforeState) or not ValidEnvironmentState(AfterState) then exit;
  if not LoadEnvironmentGroup(RootKey, SubKey, Base, Known, Trusted, Exists) or not Exists then exit;
  if not ReadEnvironmentState(RootKey, SubKey, Current) then exit;
  Key := EnvironmentOwnershipKey(SubKey, Path);
  if Current = AfterState then begin
    if Operation = 'A' then begin
      if not ReadEnvironmentOwner(RootKey, Key, Original, Added, Committed) or
        (Original <> BeforeState) or (Added <> Path) then exit;
      if not RegWriteDWordValue(RootKey, Key, 'Committed', 1) then exit;
    end else if not ForgetEnvironmentOwnership(RootKey, SubKey, Key) then exit;
  end else if Current = BeforeState then begin
    if Operation = 'A' then begin
      if not ForgetEnvironmentOwnership(RootKey, SubKey, Key) then exit;
    end else begin
      if not ReadEnvironmentOwner(RootKey, Key, Original, Added, Committed) then exit;
      if (Committed <> 1) and not RegWriteDWordValue(RootKey, Key, 'Committed', 1) then exit;
    end;
  end else begin
    { 中断期间出现第三种状态，保留用户现值并放弃这项旧归属。 }
    Trusted := False;
    if not SaveEnvironmentGroup(RootKey, SubKey, Base, Current, False) then exit;
    if not ForgetEnvironmentOwnership(RootKey, SubKey, Key) then exit;
  end;
  if not SaveEnvironmentGroup(RootKey, SubKey, Base, Current, Trusted) then exit;
  if not RegDeleteValue(RootKey, EnvironmentGroupKey(SubKey), 'Pending') then exit;
  Result := CleanEnvironmentGroup(RootKey, SubKey);
end;

function ChangeEnvironmentPath(RootKey: Integer; const SubKey, Operation, Path,
  BeforeState, AfterState: String): Boolean;
var
  Current: String;
  Applied: Boolean;
begin
  Result := False;
  if Operation = 'R' then
    if not SaveEnvironmentPending(RootKey, SubKey, Operation, Path, BeforeState, AfterState) then exit;
  if Operation = 'R' then
    if not RegWriteDWordValue(RootKey, EnvironmentOwnershipKey(SubKey, Path), 'Committed', 0) then begin
      RecoverEnvironmentPending(RootKey, SubKey);
      exit;
    end;
  { 意图持久化后再核对一次，避免覆盖准备记录期间的修改。 }
  Applied := ReadEnvironmentState(RootKey, SubKey, Current) and (Current = BeforeState);
  if Applied and (BeforeState <> AfterState) then Applied := WriteEnvironmentState(RootKey, SubKey, AfterState);
  Result := RecoverEnvironmentPending(RootKey, SubKey);
  Result := Applied and Result;
end;

function ExactEnvironmentToken(const Current, Path: String; var Position: Integer): Boolean;
var
  Value, Token: String;
begin
  Value := Copy(Current, 2, MaxInt);
  Result := EnvironmentPathCount(Value, Path, StrToInt(Copy(Current, 1, 1)), Position) = 1;
  if not Result then exit;
  Token := Copy(Value, Position, Length(Path));
  Result := (Token = Path) and
    ((Position + Length(Path) > Length(Value)) or (Copy(Value, Position + Length(Path), 1) = ';'));
end;

function WithoutEnvironmentToken(const Current, Path: String): String;
var
  Position, DeleteAt, DeleteLength: Integer;
  Value: String;
begin
  Result := Current;
  if not ExactEnvironmentToken(Current, Path, Position) then exit;
  Value := Copy(Current, 2, MaxInt);
  DeleteAt := Position;
  DeleteLength := Length(Path);
  if Position > 1 then begin
    DeleteAt := Position - 1;
    DeleteLength := DeleteLength + 1;
  end else if Length(Value) > DeleteLength then DeleteLength := DeleteLength + 1;
  Delete(Value, DeleteAt, DeleteLength);
  Result := Copy(Current, 1, 1) + Value;
end;

function ProjectionOwnerMatches(RootKey: Integer; const Key, Original, Added, Base: String;
  Keep: Boolean): Boolean;
var
  Value, OriginalValue, BaseValue: String;
  Number, StorageType: LongWord;
begin
  Result := False;
  if not RegKeyExists(RootKey, Key) then begin Result := not Keep; exit; end;
  if not EnvironmentOwnerFields(RootKey, Key, Keep) then exit;
  if RegValueExists(RootKey, Key, 'Committed') then
    if not RegQueryDWordValue(RootKey, Key, 'Committed', Number) or (Number > 1) then exit;
  if RegValueExists(RootKey, Key, 'OriginalType') then begin
    if not RegQueryDWordValue(RootKey, Key, 'OriginalType', Number) then exit;
    if (IntToStr(Number) <> Copy(Original, 1, 1)) and
      (not Keep or (IntToStr(Number) <> Copy(Base, 1, 1))) then exit;
  end;
  if RegValueExists(RootKey, Key, 'OriginalValue') then begin
    if not ReadEnvironmentValue(RootKey, Key, 'OriginalValue', Value, StorageType) or (StorageType <> 1) then exit;
    { Copy 返回 AnyString，直接比较会经过 ANSI 临时值；先存入 String 保留非 BMP 字符。 }
    OriginalValue := Copy(Original, 2, MaxInt);
    BaseValue := Copy(Base, 2, MaxInt);
    if (Value <> OriginalValue) and (not Keep or (Value <> BaseValue)) then exit;
  end;
  if RegValueExists(RootKey, Key, 'AddedPath') then
    if not ReadEnvironmentValue(RootKey, Key, 'AddedPath', Value, StorageType) or
      (StorageType <> 1) or (Value <> Added) then exit;
  Result := True;
end;

function RecoverEnvironmentProjection(RootKey: Integer; const SubKey: String): Boolean;
var
  Data, Current, Base, Actual, CountText, Key: String;
  Names, Originals, Added, Keep, ActualNames: TArrayOfString;
  ValueType: LongWord;
  I, J, Count: Integer;
  Found: Boolean;
begin
  Result := False;
  if not ReadEnvironmentValue(RootKey, EnvironmentGroupKey(SubKey), 'Projection', Data, ValueType) then exit;
  if ValueType = 0 then begin Result := True; exit; end;
  if (ValueType <> 1) or (Copy(Data, 1, 1) <> '1') then exit;
  Delete(Data, 1, 1);
  if not TakeEnvironmentField(Data, Current) or not TakeEnvironmentField(Data, Base) or
    not TakeEnvironmentField(Data, CountText) or not ValidEnvironmentState(Current) or
    not ValidEnvironmentState(Base) then exit;
  Count := StrToIntDef(CountText, -1);
  if (Count < 0) or (Count > Length(Data)) then exit;
  SetArrayLength(Names, Count); SetArrayLength(Originals, Count);
  SetArrayLength(Added, Count); SetArrayLength(Keep, Count);
  for I := 0 to Count - 1 do begin
    if not TakeEnvironmentField(Data, Names[I]) or not TakeEnvironmentField(Data, Originals[I]) or
      not TakeEnvironmentField(Data, Added[I]) or not TakeEnvironmentField(Data, Keep[I]) or
      not ValidEnvironmentState(Originals[I]) or ((Keep[I] <> '0') and (Keep[I] <> '1')) or
      (EnvironmentPathIdentity(Added[I]) = '') or (Pos(';', Added[I]) > 0) or
      (CompareText(Names[I], GetSHA256OfUnicodeString(EnvironmentPathIdentity(Added[I]))) <> 0) then exit;
    for J := 0 to I - 1 do if CompareText(Names[J], Names[I]) = 0 then exit;
  end;
  if Data <> '' then exit;
  { 第三种 PATH 或新增未知 owner 必须显式失败，不能复活已经被旧代码移除的记录。 }
  if not ReadEnvironmentState(RootKey, SubKey, Actual) or (Actual <> Current) or
    RegValueExists(RootKey, EnvironmentGroupKey(SubKey), 'Pending') then exit;
  if not EnvironmentOwnerNames(RootKey, SubKey, ActualNames) then exit;
  for I := 0 to GetArrayLength(ActualNames) - 1 do begin
    Found := False;
    for J := 0 to Count - 1 do if CompareText(ActualNames[I], Names[J]) = 0 then Found := True;
    if not Found then exit;
  end;
  for I := 0 to Count - 1 do
    if not ProjectionOwnerMatches(RootKey, EnvironmentGroupKey(SubKey) + '\' + Names[I],
      Originals[I], Added[I], Base, Keep[I] = '1') then exit;
  { 全部记录先禁用，避免旧代码把半写的 OriginalType/Value 当作有效备份。 }
  for I := 0 to Count - 1 do begin
    Key := EnvironmentGroupKey(SubKey) + '\' + Names[I];
    if RegKeyExists(RootKey, Key) then
      if not RegWriteDWordValue(RootKey, Key, 'Committed', 0) then exit;
  end;
  for I := 0 to Count - 1 do begin
    Key := EnvironmentGroupKey(SubKey) + '\' + Names[I];
    if Keep[I] = '1' then begin
      if not RegWriteDWordValue(RootKey, Key, 'OriginalType', StrToInt(Copy(Base, 1, 1))) or
        not RegWriteStringValue(RootKey, Key, 'OriginalValue', Copy(Base, 2, MaxInt)) then exit;
    end else if not ForgetEnvironmentOwnership(RootKey, SubKey, Key) then exit;
  end;
  for I := 0 to Count - 1 do
    if Keep[I] = '1' then begin
      Key := EnvironmentGroupKey(SubKey) + '\' + Names[I];
      if not RegWriteDWordValue(RootKey, Key, 'Committed', 1) then exit;
      if not ReadEnvironmentOwner(RootKey, Key, Actual, CountText, ValueType) or
        (Actual <> Base) or (CountText <> Added[I]) or (ValueType <> 1) then exit;
    end;
  { 成功移交后旧卸载器只会看到它认识的四个值，并能自行清除最后一个根键。 }
  if RegValueExists(RootKey, EnvironmentGroupKey(SubKey), 'State') then
    if not RegDeleteValue(RootKey, EnvironmentGroupKey(SubKey), 'State') then exit;
  if not RegDeleteValue(RootKey, EnvironmentGroupKey(SubKey), 'Projection') then exit;
  Result := CleanEnvironmentGroup(RootKey, SubKey);
end;

function HandoffEnvironmentOwnership(RootKey: Integer; const SubKey: String): Boolean;
var
  Names: TArrayOfString;
  Current, Base, Known, Original, Added, Data, Keep, Residue: String;
  Trusted: Boolean;
  I, Position: Integer;
  Committed: LongWord;
begin
  Result := False;
  if not EnvironmentGroupFields(RootKey, SubKey) then exit;
  if not EnvironmentOwnerNames(RootKey, SubKey, Names) then exit;
  if GetArrayLength(Names) = 0 then begin Result := CleanEnvironmentGroup(RootKey, SubKey); exit; end;
  if not ReadEnvironmentState(RootKey, SubKey, Current) or
    not EnsureEnvironmentGroup(RootKey, SubKey, Current, Base, Known, Trusted) then exit;
  Residue := Current;
  for I := 0 to GetArrayLength(Names) - 1 do begin
    if not ReadEnvironmentOwner(RootKey, EnvironmentGroupKey(SubKey) + '\' + Names[I],
      Original, Added, Committed) or (Committed <> 1) then exit;
    Residue := WithoutEnvironmentToken(Residue, Added);
  end;
  if not Trusted then Base := Residue;
  Data := '1' + EnvironmentField(Current) + EnvironmentField(Base) + EnvironmentField(IntToStr(GetArrayLength(Names)));
  for I := 0 to GetArrayLength(Names) - 1 do begin
    if not ReadEnvironmentOwner(RootKey, EnvironmentGroupKey(SubKey) + '\' + Names[I],
      Original, Added, Committed) then exit;
    Keep := '0';
    if ExactEnvironmentToken(Current, Added, Position) then Keep := '1';
    Data := Data + EnvironmentField(Names[I]) + EnvironmentField(Original) +
      EnvironmentField(Added) + EnvironmentField(Keep);
  end;
  if not RegWriteStringValue(RootKey, EnvironmentGroupKey(SubKey), 'Projection', Data) then exit;
  Result := RecoverEnvironmentProjection(RootKey, SubKey);
end;

function AddEnvironmentPath(RootKey: Integer; const SubKey, Path: String): Boolean;
var
  Current, Base, Known, Key, Installed: String;
  Trusted: Boolean;
  Position: Integer;
begin
  Result := False;
  if (EnvironmentPathIdentity(Path) = '') or (Pos(';', Path) > 0) then exit;
  if not RecoverEnvironmentPending(RootKey, SubKey) then exit;
  if not ReadEnvironmentState(RootKey, SubKey, Current) then exit;
  { 已存在项不归本次安装所有；升级不能覆盖首次记录，但仍要观察用户编辑。 }
  if EnvironmentPathCount(Copy(Current, 2, MaxInt), Path, StrToInt(Copy(Current, 1, 1)), Position) > 0 then begin
    if RegKeyExists(RootKey, EnvironmentGroupKey(SubKey)) then
      if not EnsureEnvironmentGroup(RootKey, SubKey, Current, Base, Known, Trusted) then exit;
    Result := HandoffEnvironmentOwnership(RootKey, SubKey);
    exit;
  end;
  if not EnsureEnvironmentGroup(RootKey, SubKey, Current, Base, Known, Trusted) then exit;
  Key := EnvironmentOwnershipKey(SubKey, Path);
  Installed := AppendedEnvironmentState(Current, Path);
  { 连准备 owner 的中断也有意图覆盖，恢复不依赖半写记录完整。 }
  if not SaveEnvironmentPending(RootKey, SubKey, 'A', Path, Current, Installed) then begin
    CleanEnvironmentGroup(RootKey, SubKey);
    exit;
  end;
  if not RegWriteDWordValue(RootKey, Key, 'Committed', 0) or
    not RegWriteDWordValue(RootKey, Key, 'OriginalType', StrToInt(Copy(Current, 1, 1))) or
    not RegWriteStringValue(RootKey, Key, 'OriginalValue', Copy(Current, 2, MaxInt)) or
    not RegWriteStringValue(RootKey, Key, 'AddedPath', Path) then begin
    RecoverEnvironmentPending(RootKey, SubKey);
    exit;
  end;
  Result := ChangeEnvironmentPath(RootKey, SubKey, 'A', Path, Current, Installed);
  if not Result and not RegValueExists(RootKey, EnvironmentGroupKey(SubKey), 'Pending') and
    not RegValueExists(RootKey, EnvironmentGroupKey(SubKey), 'Projection') then begin
    ForgetEnvironmentOwnership(RootKey, SubKey, Key);
    CleanEnvironmentGroup(RootKey, SubKey);
  end;
  { PATH 已提交后移交失败，保留有效 owner 供重试，不能当作准备失败清理。 }
  if Result then Result := HandoffEnvironmentOwnership(RootKey, SubKey);
end;

function RemoveEnvironmentPath(RootKey: Integer; const SubKey, Path: String): Boolean;
var
  Key, Original, AddedPath, Current, Base, Known, Updated, Value, Token: String;
  Committed, ValueType: LongWord;
  Position, Count, DeleteAt, DeleteLength: Integer;
  Trusted: Boolean;
  Names: TArrayOfString;
begin
  Result := False;
  if not RecoverEnvironmentPending(RootKey, SubKey) then exit;
  Key := EnvironmentOwnershipKey(SubKey, Path);
  { 旧版本和安装前已有项没有可验证归属，保守保留。 }
  if not RegKeyExists(RootKey, Key) then begin
    Result := HandoffEnvironmentOwnership(RootKey, SubKey);
    exit;
  end;
  if not ReadEnvironmentOwner(RootKey, Key, Original, AddedPath, Committed) or (Committed <> 1) then exit;
  if EnvironmentPathIdentity(AddedPath) <> EnvironmentPathIdentity(Path) then exit;
  if not ReadEnvironmentState(RootKey, SubKey, Current) then exit;
  if not EnsureEnvironmentGroup(RootKey, SubKey, Current, Base, Known, Trusted) then exit;
  if not EnvironmentOwnerNames(RootKey, SubKey, Names) then exit;
  Updated := Current;
  if Trusted and (GetArrayLength(Names) = 1) then Updated := Base
  else begin
    Value := Copy(Current, 2, MaxInt);
    ValueType := StrToInt(Copy(Current, 1, 1));
    Count := EnvironmentPathCount(Value, AddedPath, ValueType, Position);
    if Count = 1 then begin
      { 用户改过其它内容时，仅删除原文仍匹配的唯一新增项，保留当前类型。 }
      Token := Copy(Value, Position, Length(AddedPath));
      if (Token = AddedPath) and
        ((Position + Length(AddedPath) > Length(Value)) or
         (Copy(Value, Position + Length(AddedPath), 1) = ';')) then begin
        DeleteAt := Position;
        DeleteLength := Length(AddedPath);
        if Position > 1 then begin
          DeleteAt := Position - 1;
          DeleteLength := DeleteLength + 1;
        end else if Length(Value) > DeleteLength then DeleteLength := DeleteLength + 1;
        Delete(Value, DeleteAt, DeleteLength);
        Updated := EnvironmentState(Value, ValueType);
      end else Log('PATH 项原文已变化，保留并放弃旧归属。');
    end else if Count > 1 then Log('PATH 存在多个等价项，无法确认归属，保留用户修改。');
  end;
  Result := ChangeEnvironmentPath(RootKey, SubKey, 'R', Path, Current, Updated);
  if Result then Result := HandoffEnvironmentOwnership(RootKey, SubKey);
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
