[Code]
function EnvironmentTestCodeUnits(const Value: String): String;
var
  I: Integer;
begin
  Result := '';
  for I := 1 to Length(Value) do begin
    if I > 1 then Result := Result + ',';
    Result := Result + IntToStr(Ord(Value[I]));
  end;
end;

procedure TraceEnvironmentTestValue(const Key, Name, LabelText: String);
var
  OpenedKey: THandle;
  Status: Longint;
  Size, ValueType, ReadType: LongWord;
  Buffer, Value: String;
  ReadSuccess: Boolean;
begin
  { 只记录本轮人工夹具，禁止将真实用户 PATH 或其他注册表值写入日志。 }
  if Pos('Software\InfiniShellEnvironmentTests\{#TestId}\', Key) <> 1 then
    RaiseException('Diagnostic registry key is outside the isolated fixture');
  Status := EnvironmentOpenKey(HKCU, Key, 0, EnvironmentAccess($0001), OpenedKey);
  Log('fixture ' + LabelText + ' ' + Name + ' open=' + IntToStr(Status));
  if Status = 0 then begin
    try
      Size := 0;
      ValueType := 0;
      Status := EnvironmentQueryType(OpenedKey, Name, 0, ValueType, 0, Size);
      Log('fixture query status=' + IntToStr(Status) + ' type=' + IntToStr(ValueType) +
        ' bytes=' + IntToStr(Size));
      if (Status = 0) and (Size > 0) and (Size <= 65536) and (Size mod 2 = 0) then begin
        Buffer := StringOfChar(#0, Size div 2);
        Status := EnvironmentQueryString(OpenedKey, Name, 0, ValueType, Buffer, Size);
        Log('fixture raw status=' + IntToStr(Status) + ' bytes=' + IntToStr(Size) +
          ' length=' + IntToStr(Length(Buffer)) + ' units=' + EnvironmentTestCodeUnits(Buffer));
      end;
    finally
      EnvironmentCloseKey(OpenedKey);
    end;
  end;
  ReadSuccess := ReadEnvironmentValue(HKCU, Key, Name, Value, ReadType);
  Log('fixture helper-read success=' + IntToStr(Ord(ReadSuccess)) +
    ' type=' + IntToStr(ReadType) + ' length=' + IntToStr(Length(Value)) +
    ' units=' + EnvironmentTestCodeUnits(Value));
end;

function TraceEnvironmentTestPredicate(const LabelText: String; Passed: Boolean): Boolean;
begin
  Log('fixture predicate ' + LabelText + '=' + IntToStr(Ord(Passed)));
  Result := Passed;
end;

procedure TraceEnvironmentProjectionPredicates(const SubKey, LabelText: String);
var
  Data, Current, Base, Actual, CountText, Key, Value, OwnerOriginal, OwnerAdded: String;
  Names, Originals, Added, Keep, ActualNames, FieldNames, Children: TArrayOfString;
  ValueType, Number, StorageType, Committed: LongWord;
  I, J, Count: Integer;
  Found, Success, MatchesOriginal, MatchesBase: Boolean;
begin
  { 只复查恢复函数的读取条件，不重试恢复，也不执行禁用 owner、写入或清理。 }
  if Pos('Software\InfiniShellEnvironmentTests\{#TestId}\', SubKey) <> 1 then
    RaiseException('Projection diagnostic key is outside the isolated fixture');
  Log('fixture projection predicates ' + LabelText);
  if not TraceEnvironmentTestPredicate('projection.read',
    ReadEnvironmentValue(HKCU, EnvironmentGroupKey(SubKey), 'Projection', Data, ValueType)) then exit;
  Log('fixture projection type=' + IntToStr(ValueType));
  if ValueType = 0 then exit;
  if not TraceEnvironmentTestPredicate('projection.string-type', ValueType = 1) then exit;
  if not TraceEnvironmentTestPredicate('projection.version', Copy(Data, 1, 1) = '1') then exit;
  Delete(Data, 1, 1);
  if not TraceEnvironmentTestPredicate('parse.current', TakeEnvironmentField(Data, Current)) then exit;
  Log('fixture current length=' + IntToStr(Length(Current)) + ' units=' + EnvironmentTestCodeUnits(Current));
  if not TraceEnvironmentTestPredicate('parse.base', TakeEnvironmentField(Data, Base)) then exit;
  Log('fixture base length=' + IntToStr(Length(Base)) + ' units=' + EnvironmentTestCodeUnits(Base));
  if not TraceEnvironmentTestPredicate('parse.count', TakeEnvironmentField(Data, CountText)) then exit;
  Log('fixture count units=' + EnvironmentTestCodeUnits(CountText));
  if not TraceEnvironmentTestPredicate('current.valid-state', ValidEnvironmentState(Current)) then exit;
  if not TraceEnvironmentTestPredicate('base.valid-state', ValidEnvironmentState(Base)) then exit;
  Count := StrToIntDef(CountText, -1);
  if not TraceEnvironmentTestPredicate('count.nonnegative', Count >= 0) then exit;
  if not TraceEnvironmentTestPredicate('count.within-data', Count <= Length(Data)) then exit;
  SetArrayLength(Names, Count); SetArrayLength(Originals, Count);
  SetArrayLength(Added, Count); SetArrayLength(Keep, Count);
  for I := 0 to Count - 1 do begin
    { var 数组元素与生产调用保持一致，直接观察脚本运行时的字段传递。 }
    if not TraceEnvironmentTestPredicate('parse.name.' + IntToStr(I), TakeEnvironmentField(Data, Names[I])) then exit;
    if not TraceEnvironmentTestPredicate('parse.original.' + IntToStr(I), TakeEnvironmentField(Data, Originals[I])) then exit;
    if not TraceEnvironmentTestPredicate('parse.added.' + IntToStr(I), TakeEnvironmentField(Data, Added[I])) then exit;
    if not TraceEnvironmentTestPredicate('parse.keep.' + IntToStr(I), TakeEnvironmentField(Data, Keep[I])) then exit;
    Log('fixture owner index=' + IntToStr(I) + ' name=' + Names[I] + ' remaining=' + IntToStr(Length(Data)));
    Log('fixture original units=' + EnvironmentTestCodeUnits(Originals[I]));
    Log('fixture added units=' + EnvironmentTestCodeUnits(Added[I]) + ' keep units=' + EnvironmentTestCodeUnits(Keep[I]));
    if not TraceEnvironmentTestPredicate('original.valid-state', ValidEnvironmentState(Originals[I])) then exit;
    if not TraceEnvironmentTestPredicate('keep.valid-flag', (Keep[I] = '0') or (Keep[I] = '1')) then exit;
    if not TraceEnvironmentTestPredicate('added.nonempty', EnvironmentPathIdentity(Added[I]) <> '') then exit;
    if not TraceEnvironmentTestPredicate('added.no-delimiter', Pos(';', Added[I]) = 0) then exit;
    if not TraceEnvironmentTestPredicate('name.matches-hash',
      CompareText(Names[I], GetSHA256OfUnicodeString(EnvironmentPathIdentity(Added[I]))) = 0) then exit;
    for J := 0 to I - 1 do
      if not TraceEnvironmentTestPredicate('name.unique.' + IntToStr(J), CompareText(Names[J], Names[I]) <> 0) then exit;
  end;
  if not TraceEnvironmentTestPredicate('parse.no-trailing-data', Data = '') then exit;
  if not TraceEnvironmentTestPredicate('actual.read', ReadEnvironmentState(HKCU, SubKey, Actual)) then exit;
  Log('fixture actual units=' + EnvironmentTestCodeUnits(Actual));
  if not TraceEnvironmentTestPredicate('actual.matches-current', Actual = Current) then exit;
  if not TraceEnvironmentTestPredicate('pending.absent',
    not RegValueExists(HKCU, EnvironmentGroupKey(SubKey), 'Pending')) then exit;
  if not TraceEnvironmentTestPredicate('owners.enumerate', EnvironmentOwnerNames(HKCU, SubKey, ActualNames)) then exit;
  Log('fixture actual owner count=' + IntToStr(GetArrayLength(ActualNames)));
  for I := 0 to GetArrayLength(ActualNames) - 1 do begin
    Found := False;
    for J := 0 to Count - 1 do if CompareText(ActualNames[I], Names[J]) = 0 then Found := True;
    Log('fixture actual owner name=' + ActualNames[I]);
    if not TraceEnvironmentTestPredicate('owner.in-projection', Found) then exit;
  end;
  for I := 0 to Count - 1 do begin
    Key := EnvironmentGroupKey(SubKey) + '\' + Names[I];
    Log('fixture owner predicates index=' + IntToStr(I));
    Success := RegKeyExists(HKCU, Key);
    if not TraceEnvironmentTestPredicate('owner.exists-or-removed', Success or (Keep[I] <> '1')) then exit;
    if Success then begin
      if not TraceEnvironmentTestPredicate('owner.value-names', RegGetValueNames(HKCU, Key, FieldNames)) then exit;
      if not TraceEnvironmentTestPredicate('owner.child-names', RegGetSubkeyNames(HKCU, Key, Children)) then exit;
      Log('fixture owner value-count=' + IntToStr(GetArrayLength(FieldNames)) + ' child-count=' + IntToStr(GetArrayLength(Children)));
      if not TraceEnvironmentTestPredicate('owner.no-children', GetArrayLength(Children) = 0) then exit;
      if not TraceEnvironmentTestPredicate('owner.complete-if-kept', (Keep[I] <> '1') or (GetArrayLength(FieldNames) = 4)) then exit;
      for J := 0 to GetArrayLength(FieldNames) - 1 do begin
        Log('fixture owner field=' + FieldNames[J]);
        if not TraceEnvironmentTestPredicate('owner.known-field',
          (CompareText(FieldNames[J], 'Committed') = 0) or (CompareText(FieldNames[J], 'OriginalType') = 0) or
          (CompareText(FieldNames[J], 'OriginalValue') = 0) or (CompareText(FieldNames[J], 'AddedPath') = 0)) then exit;
      end;
      if not TraceEnvironmentTestPredicate('owner.fields', EnvironmentOwnerFields(HKCU, Key, Keep[I] = '1')) then exit;
      if RegValueExists(HKCU, Key, 'Committed') then begin
        if not TraceEnvironmentTestPredicate('committed.read', RegQueryDWordValue(HKCU, Key, 'Committed', Number)) then exit;
        Log('fixture committed=' + IntToStr(Number));
        if not TraceEnvironmentTestPredicate('committed.valid-range', Number <= 1) then exit;
      end else Log('fixture committed absent');
      if RegValueExists(HKCU, Key, 'OriginalType') then begin
        if not TraceEnvironmentTestPredicate('original-type.read', RegQueryDWordValue(HKCU, Key, 'OriginalType', Number)) then exit;
        Log('fixture original type=' + IntToStr(Number));
        MatchesOriginal := IntToStr(Number) = Copy(Originals[I], 1, 1);
        MatchesBase := IntToStr(Number) = Copy(Base, 1, 1);
        Log('fixture original-type matches-original=' + IntToStr(Ord(MatchesOriginal)) + ' matches-base=' + IntToStr(Ord(MatchesBase)));
        if not TraceEnvironmentTestPredicate('original-type.allowed', MatchesOriginal or ((Keep[I] = '1') and MatchesBase)) then exit;
      end else Log('fixture original type absent');
      if RegValueExists(HKCU, Key, 'OriginalValue') then begin
        if not TraceEnvironmentTestPredicate('original-value.read', ReadEnvironmentValue(HKCU, Key, 'OriginalValue', Value, StorageType)) then exit;
        if not TraceEnvironmentTestPredicate('original-value.string-type', StorageType = 1) then exit;
        Log('fixture stored original units=' + EnvironmentTestCodeUnits(Value));
        MatchesOriginal := Value = Copy(Originals[I], 2, MaxInt);
        MatchesBase := Value = Copy(Base, 2, MaxInt);
        Log('fixture original-value matches-original=' + IntToStr(Ord(MatchesOriginal)) + ' matches-base=' + IntToStr(Ord(MatchesBase)));
        if not TraceEnvironmentTestPredicate('original-value.allowed', MatchesOriginal or ((Keep[I] = '1') and MatchesBase)) then exit;
      end else Log('fixture original value absent');
      if RegValueExists(HKCU, Key, 'AddedPath') then begin
        if not TraceEnvironmentTestPredicate('added-path.read', ReadEnvironmentValue(HKCU, Key, 'AddedPath', Value, StorageType)) then exit;
        if not TraceEnvironmentTestPredicate('added-path.string-type', StorageType = 1) then exit;
        if not TraceEnvironmentTestPredicate('added-path.matches', Value = Added[I]) then exit;
      end else Log('fixture added path absent');
    end;
    if not TraceEnvironmentTestPredicate('owner.matches-projection',
      ProjectionOwnerMatches(HKCU, Key, Originals[I], Added[I], Base, Keep[I] = '1')) then exit;
    { 仅观察现有提交状态；这里不会执行生产函数后半段的任何写入。 }
    if Keep[I] = '1' then begin
      Success := ReadEnvironmentOwner(HKCU, Key, OwnerOriginal, OwnerAdded, Committed);
      TraceEnvironmentTestPredicate('postcondition.owner-readable', Success);
      Log('fixture postcondition original units=' + EnvironmentTestCodeUnits(OwnerOriginal));
      Log('fixture postcondition added units=' + EnvironmentTestCodeUnits(OwnerAdded) + ' committed=' + IntToStr(Committed));
      TraceEnvironmentTestPredicate('postcondition.original-matches-base', OwnerOriginal = Base);
      TraceEnvironmentTestPredicate('postcondition.added-matches', OwnerAdded = Added[I]);
      TraceEnvironmentTestPredicate('postcondition.committed', Committed = 1);
    end;
  end;
  Log('fixture projection read predicates completed; write and cleanup stages were not executed');
end;

procedure TraceEnvironmentTestState(const Key, Path, LabelText: String);
var
  OwnerKey: String;
begin
  TraceEnvironmentTestValue(Key, 'Path', LabelText);
  TraceEnvironmentTestValue(EnvironmentGroupKey(Key), 'State', LabelText);
  TraceEnvironmentTestValue(EnvironmentGroupKey(Key), 'Pending', LabelText);
  TraceEnvironmentTestValue(EnvironmentGroupKey(Key), 'Projection', LabelText);
  OwnerKey := EnvironmentOwnershipKey(Key, Path);
  TraceEnvironmentTestValue(OwnerKey, 'OriginalValue', LabelText);
  TraceEnvironmentTestValue(OwnerKey, 'AddedPath', LabelText);
  TraceEnvironmentProjectionPredicates(Key, LabelText);
end;
