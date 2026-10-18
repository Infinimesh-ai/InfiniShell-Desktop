[Setup]
AppId=InfiniShellEnvironmentTest-{#TestId}
AppName=InfiniShell Environment Test
AppVersion=1
DefaultDirName={tmp}\unused
CreateAppDir=no
Uninstallable=no
PrivilegesRequired=lowest
ArchitecturesAllowed=x64
ArchitecturesInstallIn64BitMode=x64
OutputBaseFilename=environment-test
OutputDir={#EvidenceDir}
Compression=none

#include "environment.iss"

[Code]
const
  TestRoot = 'Software\InfiniShellEnvironmentTests\{#TestId}';
  TestPath = 'C:\安装 测试\InfiniShell\bin';
  OtherPath = 'D:\TUI Tools\bin';

procedure Require(Value: Boolean; const LabelText: String);
begin
  if not Value then RaiseException('FAIL: ' + LabelText);
  if not SaveStringToFile('{#EvidenceDir}\assertions.log', 'PASS: ' + LabelText + #13#10, True) then
    RaiseException('Cannot write assertion evidence');
end;

function CaseKey(const Name: String): String;
begin
  Result := TestRoot + '\' + Name;
end;

procedure AddCase(const Name: String);
begin
  Require(AddEnvironmentPath(HKCU, CaseKey(Name), TestPath), Name + ': add');
end;

procedure RemoveCase(const Name: String);
begin
  Require(RemoveEnvironmentPath(HKCU, CaseKey(Name), TestPath), Name + ': remove');
end;

procedure RoundTrip(const Name: String);
begin
  AddCase(Name);
  RemoveCase(Name);
end;

procedure EditCase(const Name, Value: String; ValueType: LongWord);
begin
  Require(WriteEnvironmentPath(HKCU, CaseKey(Name), Value, ValueType), Name + ': user edit');
  RemoveCase(Name);
end;

procedure CheckState(const Name, Expected: String; ExpectedType: LongWord);
var
  Actual: String;
  ActualType: LongWord;
begin
  Require(ReadEnvironmentPath(HKCU, CaseKey(Name), Actual, ActualType) and
    (Actual = Expected) and (ActualType = ExpectedType), Name + ': exact raw value and type');
end;

procedure CheckOwners(const Name: String; Expected: Integer);
var
  Names: TArrayOfString;
  I: Integer;
  Committed: LongWord;
begin
  Require(EnvironmentOwnerNames(HKCU, CaseKey(Name), Names) and
    (GetArrayLength(Names) = Expected), Name + ': remaining owner count');
  for I := 0 to GetArrayLength(Names) - 1 do
    Require(RegQueryDWordValue(HKCU, EnvironmentGroupKey(CaseKey(Name)) + '\' + Names[I],
      'Committed', Committed) and (Committed = 1), Name + ': remaining owner committed');
  Require(not RegValueExists(HKCU, EnvironmentGroupKey(CaseKey(Name)), 'Pending'), Name + ': no pending intent');
  if Expected = 0 then
    Require(not RegKeyExists(HKCU, EnvironmentGroupKey(CaseKey(Name))), Name + ': shared state removed');
end;

function ProductPath(Product: Integer): String;
begin
  case Product of
    0: Result := TestPath;
    1: Result := OtherPath;
    2: Result := 'E:\第三个 产品\bin';
  end;
end;

procedure ProductOrderMatrix();
var
  Orders: TArrayOfString;
  Active: array[0..2] of Boolean;
  Kind, Order, Product, Step, Count, Removed: Integer;
  Name, Base, Expected: String;
  BaseType, InstalledType: LongWord;
begin
  SetArrayLength(Orders, 8);
  Orders[0] := 'AB'; Orders[1] := 'BA'; Orders[2] := 'ABC'; Orders[3] := 'ACB';
  Orders[4] := 'BAC'; Orders[5] := 'BCA'; Orders[6] := 'CAB'; Orders[7] := 'CBA';
  for Kind := 0 to 4 do
    for Order := 0 to 7 do begin
      Name := 'matrix-' + IntToStr(Kind) + '-' + Orders[Order];
      Base := ''; BaseType := 0;
      case Kind of
        1: BaseType := 1;
        2: BaseType := 2;
        3: begin Base := ';C:\Windows;;;D:\中文 路径;'; BaseType := 1; end;
        4: begin Base := '%SystemRoot%\System32;;%NOT_DEFINED_InfiniShell%;'; BaseType := 2; end;
      end;
      InstalledType := BaseType;
      if InstalledType = 0 then InstalledType := 2;
      Count := Length(Orders[Order]);
      Expected := Base;
      for Product := 0 to 2 do Active[Product] := False;
      for Product := 0 to Count - 1 do begin
        Require(AddEnvironmentPath(HKCU, CaseKey(Name), ProductPath(Product)), Name + ': add ' + IntToStr(Product));
        if Expected <> '' then Expected := Expected + ';';
        Expected := Expected + ProductPath(Product);
        Active[Product] := True;
        CheckState(Name, Expected, InstalledType);
        CheckOwners(Name, Product + 1);
      end;
      for Step := 1 to Count do begin
        Removed := Pos(Copy(Orders[Order], Step, 1), 'ABC') - 1;
        Require(RemoveEnvironmentPath(HKCU, CaseKey(Name), ProductPath(Removed)), Name + ': remove ' + IntToStr(Removed));
        Active[Removed] := False;
        Expected := Base;
        for Product := 0 to Count - 1 do
          if Active[Product] then begin
            if Expected <> '' then Expected := Expected + ';';
            Expected := Expected + ProductPath(Product);
          end;
        if Step = Count then CheckState(Name, Base, BaseType)
        else CheckState(Name, Expected, InstalledType);
        CheckOwners(Name, Count - Step);
      end;
    end;
end;

procedure SeedOwner(const Name, Path, BeforeState: String; Committed: LongWord);
var
  Key: String;
begin
  Key := EnvironmentOwnershipKey(CaseKey(Name), Path);
  Require(RegWriteDWordValue(HKCU, Key, 'Committed', Committed) and
    RegWriteDWordValue(HKCU, Key, 'OriginalType', StrToInt(Copy(BeforeState, 1, 1))) and
    RegWriteStringValue(HKCU, Key, 'OriginalValue', Copy(BeforeState, 2, MaxInt)) and
    RegWriteStringValue(HKCU, Key, 'AddedPath', Path), Name + ': seed prior owner');
end;

procedure PreparePending(const Name, Operation: String; Applied: Boolean);
var
  BeforeState, AfterState: String;
begin
  if Operation = 'A' then begin
    BeforeState := '0'; AfterState := '2' + TestPath;
  end else begin
    BeforeState := '2' + TestPath; AfterState := '0';
  end;
  Require(SaveEnvironmentGroup(HKCU, CaseKey(Name), '0', BeforeState, True), Name + ': seed shared state');
  SeedOwner(Name, TestPath, '0', 0);
  Require(SaveEnvironmentPending(HKCU, CaseKey(Name), Operation, TestPath, BeforeState, AfterState),
    Name + ': persist pending boundary');
  if Applied then begin
    if AfterState <> '0' then Require(WriteEnvironmentState(HKCU, CaseKey(Name), AfterState), Name + ': write after')
    else if RegValueExists(HKCU, CaseKey(Name), 'Path') then
      Require(RegDeleteValue(HKCU, CaseKey(Name), 'Path'), Name + ': delete after');
  end else if BeforeState <> '0' then
    Require(WriteEnvironmentState(HKCU, CaseKey(Name), BeforeState), Name + ': write before');
end;

procedure MoreCases();
var
  Name: String;
  I: Integer;
begin
  AddCase('cross-upgrade');
  Require(AddEnvironmentPath(HKCU, CaseKey('cross-upgrade'), OtherPath), 'cross-upgrade: add B');
  AddCase('cross-upgrade');
  RemoveCase('cross-upgrade');
  AddCase('cross-upgrade');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('cross-upgrade'), OtherPath), 'cross-upgrade: remove B');
  RemoveCase('cross-upgrade');
  CheckState('cross-upgrade', '', 0);

  for I := 0 to 5 do begin
    Name := 'cross-edit-' + IntToStr(I);
    AddCase(Name);
    Require(AddEnvironmentPath(HKCU, CaseKey(Name), OtherPath), Name + ': add B');
    RemoveCase(Name);
    case I of
      0: Require(WriteEnvironmentPath(HKCU, CaseKey(Name), '', 1), Name + ': user empty SZ');
      1: Require(WriteEnvironmentPath(HKCU, CaseKey(Name), '', 2), Name + ': user empty EXPAND');
      2: Require(RegDeleteValue(HKCU, CaseKey(Name), 'Path'), Name + ': user deletes value');
      3: Require(WriteEnvironmentPath(HKCU, CaseKey(Name), OtherPath, 1), Name + ': user changes only type');
      4: Require(WriteEnvironmentPath(HKCU, CaseKey(Name), OtherPath + ';D:\User', 2), Name + ': user appends');
      5: Require(WriteEnvironmentPath(HKCU, CaseKey(Name), '', 1), Name + ': user empties before C');
    end;
    if I = 5 then Require(AddEnvironmentPath(HKCU, CaseKey(Name), ProductPath(2)), Name + ': add C after user edit');
    Require(RemoveEnvironmentPath(HKCU, CaseKey(Name), OtherPath), Name + ': remove B');
    if I = 5 then Require(RemoveEnvironmentPath(HKCU, CaseKey(Name), ProductPath(2)), Name + ': remove C');
  end;

  SeedOwner('r4-single', TestPath, '0', 1);
  Require(WriteEnvironmentPath(HKCU, CaseKey('r4-single'), TestPath, 2), 'r4-single: installed');
  RemoveCase('r4-single');
  SeedOwner('r4-chain', TestPath, '0', 1);
  SeedOwner('r4-chain', OtherPath, '2' + TestPath, 1);
  SeedOwner('r4-chain', ProductPath(2), '2' + TestPath + ';' + OtherPath, 1);
  Require(WriteEnvironmentPath(HKCU, CaseKey('r4-chain'), TestPath + ';' + OtherPath + ';' + ProductPath(2), 2), 'r4-chain: installed');
  RemoveCase('r4-chain');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('r4-chain'), OtherPath), 'r4-chain: remove B');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('r4-chain'), ProductPath(2)), 'r4-chain: remove C');
  SeedOwner('r4-incomplete', OtherPath, '2' + TestPath, 1);
  Require(WriteEnvironmentPath(HKCU, CaseKey('r4-incomplete'), OtherPath, 2), 'r4-incomplete: missing history');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('r4-incomplete'), OtherPath), 'r4-incomplete: conservative removal');
  AddCase('old-helper-interleaving');
  Require(AddEnvironmentPath(HKCU, CaseKey('old-helper-interleaving'), OtherPath), 'old-helper-interleaving: add B');
  { 模拟旧卸载器只更新 PATH 和自己的记录，不更新共享状态。 }
  Require(WriteEnvironmentPath(HKCU, CaseKey('old-helper-interleaving'), OtherPath, 2), 'old-helper-interleaving: external PATH change');
  Require(ForgetEnvironmentOwnership(HKCU, CaseKey('old-helper-interleaving'),
    EnvironmentOwnershipKey(CaseKey('old-helper-interleaving'), TestPath)), 'old-helper-interleaving: old owner removed');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('old-helper-interleaving'), OtherPath), 'old-helper-interleaving: conservative removal');

  PreparePending('pending-add-before', 'A', False);
  Require(RecoverEnvironmentPending(HKCU, CaseKey('pending-add-before')), 'pending-add-before: rollback prepared ownership');
  PreparePending('pending-add-after', 'A', True);
  Require(RecoverEnvironmentPending(HKCU, CaseKey('pending-add-after')), 'pending-add-after: finish ownership');
  CheckOwners('pending-add-after', 1);
  RemoveCase('pending-add-after');
  PreparePending('pending-remove-before', 'R', False);
  Require(RecoverEnvironmentPending(HKCU, CaseKey('pending-remove-before')), 'pending-remove-before: restore committed ownership');
  CheckOwners('pending-remove-before', 1);
  RemoveCase('pending-remove-before');
  PreparePending('pending-remove-after', 'R', True);
  Require(RegDeleteValue(HKCU, EnvironmentOwnershipKey(CaseKey('pending-remove-after'), TestPath), 'Committed'),
    'pending-remove-after: interrupted partial cleanup');
  Require(RecoverEnvironmentPending(HKCU, CaseKey('pending-remove-after')), 'pending-remove-after: finish partial cleanup');
  PreparePending('pending-user-edit', 'R', True);
  Require(WriteEnvironmentPath(HKCU, CaseKey('pending-user-edit'), '', 1), 'pending-user-edit: user recreates empty value');
  Require(RecoverEnvironmentPending(HKCU, CaseKey('pending-user-edit')), 'pending-user-edit: preserve third state');

  Require(RegWriteStringValue(HKCU, EnvironmentGroupKey(CaseKey('invalid-state')), 'State', '11oops'), 'invalid-state: malformed fixture');
  Require(not AddEnvironmentPath(HKCU, CaseKey('invalid-state'), TestPath), 'invalid-state: reject without PATH mutation');
  Require(RegWriteStringValue(HKCU, EnvironmentGroupKey(CaseKey('invalid-pending')), 'Pending', '1A999:x'), 'invalid-pending: malformed fixture');
  Require(not AddEnvironmentPath(HKCU, CaseKey('invalid-pending'), TestPath), 'invalid-pending: reject without PATH mutation');

  for I := 0 to 1 do begin
    Name := 'pending-owner-prepare-' + IntToStr(I);
    Require(SaveEnvironmentGroup(HKCU, CaseKey(Name), '0', '0', True), Name + ': group before prepare');
    Require(SaveEnvironmentPending(HKCU, CaseKey(Name), 'A', TestPath, '0', '2' + TestPath), Name + ': intent before owner');
    if I = 1 then Require(RegWriteDWordValue(HKCU, EnvironmentOwnershipKey(CaseKey(Name), TestPath),
      'Committed', 0), Name + ': interrupted partial owner');
    Require(AddEnvironmentPath(HKCU, CaseKey(Name), OtherPath), Name + ': other product recovers preparation');
    Require(RemoveEnvironmentPath(HKCU, CaseKey(Name), OtherPath), Name + ': other product restores absence');
    CheckState(Name, '', 0);
    CheckOwners(Name, 0);
  end;
  Require(SaveEnvironmentGroup(HKCU, CaseKey('stale-empty-group'), '0', '0', False), 'stale-empty-group: interrupted final cleanup');
  RoundTrip('stale-empty-group');
  CheckState('stale-empty-group', '', 0);
  CheckOwners('stale-empty-group', 0);
  AddCase('owner-delete-denied');
  Require(AddEnvironmentPath(HKCU, CaseKey('owner-delete-denied'), OtherPath), 'owner-delete-denied: add B');
  AddCase('group-delete-denied');

  PreparePending('cleanup-denied-retry', 'R', True);
  PreparePending('state-denied-retry', 'R', True);
  Require(SaveEnvironmentGroup(HKCU, CaseKey('shared-write-denied'), '0', '0', True), 'shared-write-denied: prepare state');
  AddCase('pending-write-denied');
end;

procedure PrepareProjection(const Name: String);
var
  Names: TArrayOfString;
  Data, Original, Added: String;
  I: Integer;
  Committed: LongWord;
begin
  SeedOwner(Name, TestPath, '0', 1);
  SeedOwner(Name, OtherPath, '2' + TestPath, 1);
  Require(WriteEnvironmentPath(HKCU, CaseKey(Name), TestPath + ';' + OtherPath, 2), Name + ': original installed path');
  Require(SaveEnvironmentGroup(HKCU, CaseKey(Name), '0', '2' + TestPath + ';' + OtherPath, True), Name + ': projection state');
  Require(EnvironmentOwnerNames(HKCU, CaseKey(Name), Names), Name + ': projection owners');
  Data := '1' + EnvironmentField('2' + TestPath + ';' + OtherPath) + EnvironmentField('0') + EnvironmentField('2');
  for I := 0 to 1 do begin
    Require(ReadEnvironmentOwner(HKCU, EnvironmentGroupKey(CaseKey(Name)) + '\' + Names[I],
      Original, Added, Committed), Name + ': projection snapshot');
    Data := Data + EnvironmentField(Names[I]) + EnvironmentField(Original) + EnvironmentField(Added) + EnvironmentField('1');
  end;
  Require(RegWriteStringValue(HKCU, EnvironmentGroupKey(CaseKey(Name)), 'Projection', Data), Name + ': durable projection intent');
end;

procedure ProjectionCases();
var
  Names: TArrayOfString;
  Name, Key: String;
  I: Integer;
begin
  AddCase('owner-enum-denied');
  PreparePending('projection-create-denied', 'A', True);
  Require(RecoverEnvironmentPending(HKCU, CaseKey('projection-create-denied')), 'projection-create-denied: commit PATH before handoff');
  PrepareProjection('projection-second-denied');
  PrepareProjection('projection-cleanup-denied');
  PrepareProjection('projection-half-write');
  Key := EnvironmentOwnershipKey(CaseKey('projection-half-write'), OtherPath);
  Require(RegWriteDWordValue(HKCU, Key, 'Committed', 0) and
    RegWriteDWordValue(HKCU, Key, 'OriginalType', 0), 'projection-half-write: interruption between type and value');
  Require(RecoverEnvironmentProjection(HKCU, CaseKey('projection-half-write')), 'projection-half-write: finish mixed fields');
  PrepareProjection('projection-third-path');
  Require(WriteEnvironmentPath(HKCU, CaseKey('projection-third-path'), TestPath + ';' + OtherPath + ';D:\User', 2),
    'projection-third-path: user change during interruption');
  Require(not RecoverEnvironmentProjection(HKCU, CaseKey('projection-third-path')), 'projection-third-path: fail without overwriting user change');
  PrepareProjection('projection-old-remove');
  PrepareProjection('projection-old-add');

  for I := 0 to 2 do begin
    Name := 'native-r5-state-' + IntToStr(I);
    SeedOwner(Name, TestPath, '0', 1);
    SeedOwner(Name, OtherPath, '2' + TestPath, 1);
    Require(WriteEnvironmentPath(HKCU, CaseKey(Name), TestPath + ';' + OtherPath, 2), Name + ': current installed');
    Key := '2' + TestPath + ';' + OtherPath;
    if I = 1 then Key := '2' + TestPath;
    Require(SaveEnvironmentGroup(HKCU, CaseKey(Name), '0', Key, I <> 2), Name + ': prior R5 state');
  end;

  for I := 0 to 1 do begin
    Name := 'native-anchor-ambiguous-' + IntToStr(I);
    SeedOwner(Name, TestPath, '0', 1);
    SeedOwner(Name, OtherPath, '2', 1);
    Require(WriteEnvironmentPath(HKCU, CaseKey(Name), TestPath + ';' + OtherPath, 2), Name + ': two typed anchors');
  end;

  Require(RegWriteStringValue(HKCU, EnvironmentGroupKey(CaseKey('invalid-root-value')), 'Unknown', 'preserve'), 'invalid-root-value: fixture');
  Require(not AddEnvironmentPath(HKCU, CaseKey('invalid-root-value'), TestPath), 'invalid-root-value: reject before mutation');
  for I := 0 to 5 do begin
    Name := 'invalid-owner-' + IntToStr(I);
    SeedOwner(Name, TestPath, '0', 1);
    Require(WriteEnvironmentPath(HKCU, CaseKey(Name), TestPath, 2), Name + ': existing PATH');
    Key := EnvironmentOwnershipKey(CaseKey(Name), TestPath);
    case I of
      0: Require(RegWriteStringValue(HKCU, Key, 'Unknown', 'preserve'), Name + ': unknown value');
      1: Require(RegWriteStringValue(HKCU, Key + '\Unknown', 'Value', 'preserve'), Name + ': unknown child');
      2: Require(RegWriteDWordValue(HKCU, Key, 'Committed', 0), Name + ': uncommitted');
      3: Require(RegWriteStringValue(HKCU, Key, 'Committed', '1'), Name + ': wrong committed storage type');
      4: Require(RegWriteStringValue(HKCU, Key, 'AddedPath', OtherPath), Name + ': hash mismatch');
      5: Require(RegWriteExpandStringValue(HKCU, Key, 'OriginalValue', ''), Name + ': wrong original storage type');
    end;
    Require(not AddEnvironmentPath(HKCU, CaseKey(Name), OtherPath), Name + ': reject invalid ownership');
    CheckState(Name, TestPath, 2);
  end;
end;

procedure DeniedPhase();
var
  Committed: LongWord;
begin
  Require(not AddEnvironmentPath(HKCU, CaseKey('owner-enum-denied'), OtherPath), 'owner-enum-denied: reject unreadable owner set');
  CheckState('owner-enum-denied', TestPath, 2);
  Require(not AddEnvironmentPath(HKCU, CaseKey('projection-create-denied'), TestPath), 'projection-create-denied: reject handoff write');
  Require(RegQueryDWordValue(HKCU, EnvironmentOwnershipKey(CaseKey('projection-create-denied'), TestPath),
    'Committed', Committed) and (Committed = 1), 'projection-create-denied: preserve committed owner after PATH success');
  CheckState('projection-create-denied', TestPath, 2);
  Require(not RecoverEnvironmentProjection(HKCU, CaseKey('projection-second-denied')), 'projection-second-denied: second owner write denied');
  Require(RegQueryDWordValue(HKCU, EnvironmentOwnershipKey(CaseKey('projection-second-denied'), OtherPath),
    'Committed', Committed) and (Committed = 0), 'projection-second-denied: earlier owner disabled');
  Require(not RecoverEnvironmentProjection(HKCU, CaseKey('projection-cleanup-denied')), 'projection-cleanup-denied: final log cleanup denied');
  Require(RegValueExists(HKCU, EnvironmentGroupKey(CaseKey('projection-cleanup-denied')), 'Projection'), 'projection-cleanup-denied: keep retry log');
  Require(not RecoverEnvironmentProjection(HKCU, CaseKey('projection-old-remove')), 'projection-old-remove: reject changed old owner set');
  Require(not RecoverEnvironmentProjection(HKCU, CaseKey('projection-old-add')), 'projection-old-add: reject new legacy owner');
  Require(not RemoveEnvironmentPath(HKCU, CaseKey('owner-delete-denied'), TestPath), 'owner-delete-denied: refuse false success');
  CheckState('owner-delete-denied', OtherPath, 2);
  Require(RegKeyExists(HKCU, EnvironmentOwnershipKey(CaseKey('owner-delete-denied'), TestPath)) and
    not RegValueExists(HKCU, EnvironmentOwnershipKey(CaseKey('owner-delete-denied'), TestPath), 'Committed') and
    RegValueExists(HKCU, EnvironmentGroupKey(CaseKey('owner-delete-denied')), 'Pending'),
    'owner-delete-denied: empty owner retains recoverable intent');
  Require(not RemoveEnvironmentPath(HKCU, CaseKey('group-delete-denied'), TestPath), 'group-delete-denied: failed final group cleanup');
  CheckState('group-delete-denied', '', 0);
  Require(not RecoverEnvironmentPending(HKCU, CaseKey('cleanup-denied-retry')), 'cleanup-denied-retry: cleanup denied');
  Require(RegQueryDWordValue(HKCU, EnvironmentOwnershipKey(CaseKey('cleanup-denied-retry'), TestPath),
    'Committed', Committed) and (Committed = 0), 'cleanup-denied-retry: removed PATH has no committed owner');
  CheckState('cleanup-denied-retry', '', 0);
  Require(not RecoverEnvironmentPending(HKCU, CaseKey('state-denied-retry')), 'state-denied-retry: state update denied');
  Require(not RegValueExists(HKCU, EnvironmentOwnershipKey(CaseKey('state-denied-retry'), TestPath), 'Committed'),
    'state-denied-retry: cleaned owner not falsely committed');
  CheckState('state-denied-retry', '', 0);
  Require(not AddEnvironmentPath(HKCU, CaseKey('shared-write-denied'), TestPath), 'shared-write-denied: pending write denied');
  CheckState('shared-write-denied', '', 0);
  Require(not RemoveEnvironmentPath(HKCU, CaseKey('pending-write-denied'), TestPath), 'pending-write-denied: removal blocked');
  CheckState('pending-write-denied', TestPath, 2);
end;

procedure RetryPhase();
begin
  AddCase('owner-enum-denied');
  AddCase('projection-create-denied');
  Require(RecoverEnvironmentProjection(HKCU, CaseKey('projection-second-denied')), 'projection-second-denied: retry complete');
  Require(RecoverEnvironmentProjection(HKCU, CaseKey('projection-cleanup-denied')), 'projection-cleanup-denied: retry complete');
  RemoveCase('owner-delete-denied');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('owner-delete-denied'), OtherPath), 'owner-delete-denied: remove B after recovery');
  CheckState('owner-delete-denied', '', 0);
  CheckOwners('owner-delete-denied', 0);
  RemoveCase('group-delete-denied');
  CheckOwners('group-delete-denied', 0);

  Require(RecoverEnvironmentPending(HKCU, CaseKey('cleanup-denied-retry')), 'cleanup-denied-retry: retry after ACL restored');
  CheckOwners('cleanup-denied-retry', 0);
  Require(RecoverEnvironmentPending(HKCU, CaseKey('state-denied-retry')), 'state-denied-retry: retry after ACL restored');
  CheckOwners('state-denied-retry', 0);
  RoundTrip('shared-write-denied');
  RemoveCase('pending-write-denied');
end;

function InitializeSetup(): Boolean;
var
  Identity, Phase: String;
begin
  { 返回 False，不进入安装流程；所有读写仅指向本轮随机测试键。 }
  Result := False;
  Require(RegQueryStringValue(HKCU, TestRoot, 'TestId', Identity) and
    (Identity = '{#TestId}'), 'unique isolated HKCU root');

  Phase := ExpandConstant('{param:phase|main}');
  if Phase = 'denied' then begin
    DeniedPhase();
    Require(SaveStringToFile('{#EvidenceDir}\completed-denied.txt', '{#TestId}', False), 'denied completion receipt');
    exit;
  end;
  if Phase = 'retry' then begin
    RetryPhase();
    Require(SaveStringToFile('{#EvidenceDir}\completed-retry.txt', '{#TestId}', False), 'retry completion receipt');
    exit;
  end;

  RoundTrip('missing');
  RoundTrip('empty-sz');
  RoundTrip('empty-expand');
  RoundTrip('plain');
  RoundTrip('separators');
  RoundTrip('expand');
  RoundTrip('delimiter-only');
  RoundTrip('preexisting');
  RoundTrip('preexisting-equivalent');
  RoundTrip('preexisting-duplicates');
  AddCase('preexisting-expand');
  Require(not RegKeyExists(HKCU, EnvironmentOwnershipKey(CaseKey('preexisting-expand'), TestPath)),
    'preexisting-expand: no ownership of equivalent expanded token');
  RemoveCase('preexisting-expand');
  AddCase('unexpanded-sz');
  Require(RegKeyExists(HKCU, EnvironmentOwnershipKey(CaseKey('unexpanded-sz'), TestPath)),
    'unexpanded-sz: literal percent token is not expanded');
  RemoveCase('unexpanded-sz');
  RemoveCase('legacy-no-owner');

  AddCase('upgrade');
  Require(AddEnvironmentPath(HKCU, CaseKey('upgrade'), Uppercase(TestPath)), 'upgrade: same path casing');
  RemoveCase('upgrade');
  AddCase('upgrade-edited');
  Require(WriteEnvironmentPath(HKCU, CaseKey('upgrade-edited'), 'D:\Added;;C:\Windows;;' + TestPath, 1),
    'upgrade-edited: user edit');
  AddCase('upgrade-edited');
  RemoveCase('upgrade-edited');

  AddCase('edited-append');
  EditCase('edited-append', 'C:\Windows;;' + TestPath + ';D:\Added;;', 1);
  AddCase('edited-prefix');
  EditCase('edited-prefix', 'D:\Added;;C:\Windows;;' + TestPath, 1);
  AddCase('edited-replace');
  EditCase('edited-replace', 'C:\Changed;' + TestPath + ';D:\Added', 1);
  AddCase('edited-reorder');
  EditCase('edited-reorder', TestPath + ';D:\Added;C:\Windows', 1);
  AddCase('edited-type-expand');
  EditCase('edited-type-expand', 'C:\Windows;;' + TestPath, 2);
  AddCase('edited-type-sz');
  EditCase('edited-type-sz', '%SystemRoot%\System32;;' + TestPath, 1);
  AddCase('edited-delete');
  Require(RegDeleteValue(HKCU, CaseKey('edited-delete'), 'Path'), 'edited-delete: user removal');
  RemoveCase('edited-delete');
  AddCase('edited-empty');
  EditCase('edited-empty', '', 1);
  AddCase('edited-duplicate');
  EditCase('edited-duplicate', 'C:\Windows;;' + TestPath + ';' + TestPath, 1);
  AddCase('edited-quoted');
  EditCase('edited-quoted', 'C:\Windows;"' + TestPath + '"', 1);
  AddCase('edited-trailing-slash');
  EditCase('edited-trailing-slash', 'C:\Windows;' + TestPath + '\', 1);
  AddCase('edited-longer');
  EditCase('edited-longer', 'C:\Windows;' + TestPath + '-other', 1);
  AddCase('missing-user-added');
  EditCase('missing-user-added', TestPath + ';D:\Added', 2);

  AddCase('two-products');
  Require(AddEnvironmentPath(HKCU, CaseKey('two-products'), OtherPath), 'two-products: TUI add');
  RemoveCase('two-products');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('two-products'), OtherPath), 'two-products: TUI remove');
  AddCase('tui-migration');
  RemoveCase('tui-migration');
  Require(AddEnvironmentPath(HKCU, CaseKey('tui-migration'), OtherPath), 'tui-migration: new bin');
  Require(RemoveEnvironmentPath(HKCU, CaseKey('tui-migration'), OtherPath), 'tui-migration: uninstall');

  Require(not AddEnvironmentPath(HKCU, CaseKey('wrong-type'), TestPath), 'wrong-type: rejected');
  Require(not AddEnvironmentPath(HKCU, CaseKey('read-denied'), TestPath), 'read-denied: rejected');
  Require(not AddEnvironmentPath(HKCU, CaseKey('write-denied'), TestPath), 'write-denied: rejected');
  Require(not AddEnvironmentPath(HKCU, CaseKey('write-denied-missing'), TestPath), 'write-denied-missing: rejected');
  CheckOwners('write-denied-missing', 0);
  Require(not AddEnvironmentPath(HKCU, CaseKey('record-denied'), TestPath), 'record-denied: rejected');
  Require(not RemoveEnvironmentPath(HKCU, CaseKey('remove-denied'), TestPath), 'remove-denied: rejected');
  Require(not RemoveEnvironmentPath(HKCU, CaseKey('ownership-remove-denied'), TestPath),
    'ownership-remove-denied: reject before changing PATH');

  ProductOrderMatrix();
  MoreCases();
  ProjectionCases();

  Require(SaveStringToFile('{#EvidenceDir}\completed.txt', '{#TestId}', False), 'completion receipt');
end;
