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
end;
