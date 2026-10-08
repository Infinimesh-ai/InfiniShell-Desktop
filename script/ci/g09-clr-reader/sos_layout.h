#pragma once

// 仅复制固定官方 dacprivate.h 的五种输出布局，不包含 CLR 内部 FieldDesc 布局。
// 来源、原件摘要及许可证见 sources.safe.json；限 Windows x64 的默认 MSVC 对齐。
enum DacpObjectType { OBJ_STRING = 0, OBJ_FREE, OBJ_OBJECT, OBJ_ARRAY, OBJ_OTHER };
struct DacpObjectData {
  CLRDATA_ADDRESS MethodTable = 0;
  DacpObjectType ObjectType = OBJ_STRING;
  ULONG64 Size = 0;
  CLRDATA_ADDRESS ElementTypeHandle = 0;
  CorElementType ElementType = 0;
  DWORD dwRank = 0;
  ULONG64 dwNumComponents = 0;
  ULONG64 dwComponentSize = 0;
  CLRDATA_ADDRESS ArrayDataPtr = 0;
  CLRDATA_ADDRESS ArrayBoundsPtr = 0;
  CLRDATA_ADDRESS ArrayLowerBoundsPtr = 0;
  CLRDATA_ADDRESS RCW = 0;
  CLRDATA_ADDRESS CCW = 0;
};
struct DacpUsefulGlobalsData {
  CLRDATA_ADDRESS ArrayMethodTable = 0;
  CLRDATA_ADDRESS StringMethodTable = 0;
  CLRDATA_ADDRESS ObjectMethodTable = 0;
  CLRDATA_ADDRESS ExceptionMethodTable = 0;
  CLRDATA_ADDRESS FreeMethodTable = 0;
};
struct DacpFieldDescData {
  CorElementType Type = 0;
  CorElementType sigType = 0;
  CLRDATA_ADDRESS MTOfType = 0;
  CLRDATA_ADDRESS ModuleOfType = 0;
  mdTypeDef TokenOfType = 0;
  mdFieldDef mb = 0;
  CLRDATA_ADDRESS MTOfEnclosingClass = 0;
  DWORD dwOffset = 0;
  BOOL bIsThreadLocal = FALSE;
  BOOL bIsContextLocal = FALSE;
  BOOL bIsStatic = FALSE;
  CLRDATA_ADDRESS NextField = 0;
};
struct DacpMethodTableFieldData {
  WORD wNumInstanceFields = 0;
  WORD wNumStaticFields = 0;
  WORD wNumThreadStaticFields = 0;
  CLRDATA_ADDRESS FirstField = 0;
  WORD wContextStaticOffset = 0;
  WORD wContextStaticsSize = 0;
};
struct DacpMethodTableData {
  BOOL bIsFree = FALSE;
  CLRDATA_ADDRESS Module = 0;
  CLRDATA_ADDRESS Class = 0;
  CLRDATA_ADDRESS ParentMethodTable = 0;
  WORD wNumInterfaces = 0;
  WORD wNumMethods = 0;
  WORD wNumVtableSlots = 0;
  WORD wNumVirtuals = 0;
  DWORD BaseSize = 0;
  DWORD ComponentSize = 0;
  mdTypeDef cl = 0;
  DWORD dwAttrClass = 0;
  BOOL bIsShared = FALSE;
  BOOL bIsDynamic = FALSE;
  BOOL bContainsPointers = FALSE;
};

static_assert(sizeof(DacpObjectType) == 4 && sizeof(CorElementType) == 4);
static_assert(sizeof(DacpObjectData) == 0x60);
static_assert(offsetof(DacpObjectData, MethodTable) == 0x00);
static_assert(offsetof(DacpObjectData, ObjectType) == 0x08);
static_assert(offsetof(DacpObjectData, Size) == 0x10);
static_assert(sizeof(DacpUsefulGlobalsData) == 0x28);
static_assert(offsetof(DacpUsefulGlobalsData, ObjectMethodTable) == 0x10);
static_assert(offsetof(DacpUsefulGlobalsData, ExceptionMethodTable) == 0x18);
static_assert(sizeof(DacpFieldDescData) == 0x40);
static_assert(offsetof(DacpFieldDescData, Type) == 0x00);
static_assert(offsetof(DacpFieldDescData, sigType) == 0x04);
static_assert(offsetof(DacpFieldDescData, ModuleOfType) == 0x10);
static_assert(offsetof(DacpFieldDescData, mb) == 0x1c);
static_assert(offsetof(DacpFieldDescData, MTOfEnclosingClass) == 0x20);
static_assert(offsetof(DacpFieldDescData, dwOffset) == 0x28);
static_assert(offsetof(DacpFieldDescData, bIsStatic) == 0x34);
static_assert(offsetof(DacpFieldDescData, NextField) == 0x38);
static_assert(sizeof(DacpMethodTableFieldData) == 0x18);
static_assert(offsetof(DacpMethodTableFieldData, FirstField) == 0x08);
static_assert(sizeof(DacpMethodTableData) == 0x48);
static_assert(offsetof(DacpMethodTableData, Module) == 0x08);
static_assert(offsetof(DacpMethodTableData, ParentMethodTable) == 0x18);
static_assert(offsetof(DacpMethodTableData, BaseSize) == 0x28);
static_assert(offsetof(DacpMethodTableData, cl) == 0x30);
