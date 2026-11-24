use std::collections::HashMap;
use std::fmt;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct TypeId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct StructId(pub usize);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct EnumId(pub usize);

pub const TY_VOID: TypeId = TypeId(0);
pub const TY_BOOL: TypeId = TypeId(1);
pub const TY_I8: TypeId = TypeId(2);
pub const TY_I16: TypeId = TypeId(3);
pub const TY_I32: TypeId = TypeId(4);
pub const TY_I64: TypeId = TypeId(5);
pub const TY_U8: TypeId = TypeId(6);
pub const TY_U16: TypeId = TypeId(7);
pub const TY_U32: TypeId = TypeId(8);
pub const TY_U64: TypeId = TypeId(9);
pub const TY_F32: TypeId = TypeId(10);
pub const TY_F64: TypeId = TypeId(11);
pub const TY_PTR: TypeId = TypeId(12);
pub const TY_CSTR: TypeId = TypeId(13);
pub const TY_STR: TypeId = TypeId(14);

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum TypeKind {
    Void,
    Bool,
    I8,
    I16,
    I32,
    I64,
    U8,
    U16,
    U32,
    U64,
    F32,
    F64,
    OpaquePtr,
    CStr,
    Str,
    Ptr(TypeId),
    Array(TypeId, u64),
    Slice(TypeId),
    Fn(Vec<TypeId>, TypeId),
    Struct(StructId),
    Enum(EnumId),
}

#[derive(Debug, Clone)]
pub struct FieldInfo {
    pub name: String,
    pub ty: TypeId,
    pub offset: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct StructInfo {
    pub id: StructId,
    pub name: String,
    pub c_name: String,
    pub fields: Vec<FieldInfo>,
    pub size: u32,
    pub align: u32,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct EnumInfo {
    pub id: EnumId,
    pub name: String,
    pub repr: TypeId,
    pub variants: HashMap<String, i64>,
}

#[derive(Debug, Clone, Copy)]
pub struct Layout {
    pub size: u32,
    pub align: u32,
}

pub struct TypeArena {
    types: Vec<TypeKind>,
    layouts: Vec<Layout>,
    intern: HashMap<TypeKind, TypeId>,
    pub structs: Vec<StructInfo>,
    pub enums: Vec<EnumInfo>,
}

impl TypeArena {
    pub fn new() -> Self {
        let mut arena = Self {
            types: Vec::new(),
            layouts: Vec::new(),
            intern: HashMap::new(),
            structs: Vec::new(),
            enums: Vec::new(),
        };
        let builtins = [
            (TypeKind::Void, Layout { size: 0, align: 1 }),
            (TypeKind::Bool, Layout { size: 1, align: 1 }),
            (TypeKind::I8, Layout { size: 1, align: 1 }),
            (TypeKind::I16, Layout { size: 2, align: 2 }),
            (TypeKind::I32, Layout { size: 4, align: 4 }),
            (TypeKind::I64, Layout { size: 8, align: 8 }),
            (TypeKind::U8, Layout { size: 1, align: 1 }),
            (TypeKind::U16, Layout { size: 2, align: 2 }),
            (TypeKind::U32, Layout { size: 4, align: 4 }),
            (TypeKind::U64, Layout { size: 8, align: 8 }),
            (TypeKind::F32, Layout { size: 4, align: 4 }),
            (TypeKind::F64, Layout { size: 8, align: 8 }),
            (TypeKind::OpaquePtr, Layout { size: 8, align: 8 }),
            (TypeKind::CStr, Layout { size: 8, align: 8 }),
            (TypeKind::Str, Layout { size: 16, align: 8 }),
        ];
        for (idx, (kind, layout)) in builtins.into_iter().enumerate() {
            arena.intern.insert(kind.clone(), TypeId(idx));
            arena.types.push(kind);
            arena.layouts.push(layout);
        }
        arena
    }

    pub fn kind(&self, id: TypeId) -> &TypeKind {
        &self.types[id.0]
    }

    pub fn layout(&self, id: TypeId) -> Layout {
        self.layouts[id.0]
    }

    pub fn intern(&mut self, kind: TypeKind) -> TypeId {
        if let Some(&id) = self.intern.get(&kind) {
            return id;
        }
        let layout = self.compute_layout(&kind);
        let id = TypeId(self.types.len());
        self.types.push(kind.clone());
        self.layouts.push(layout);
        self.intern.insert(kind, id);
        id
    }

    pub fn new_struct(&mut self, name: String, c_name: String) -> StructId {
        let id = StructId(self.structs.len());
        self.structs.push(StructInfo {
            id,
            name,
            c_name,
            fields: Vec::new(),
            size: 0,
            align: 1,
        });
        id
    }

    pub fn finalize_struct(&mut self, id: StructId, fields: Vec<(String, TypeId)>) {
        let mut offset = 0u32;
        let mut max_align = 1u32;
        let mut laid_out = Vec::with_capacity(fields.len());
        for (fname, fty) in fields {
            let l = self.layout(fty);
            let align = l.align.max(1);
            max_align = max_align.max(align);
            offset = align_up(offset, align);
            laid_out.push(FieldInfo {
                name: fname,
                ty: fty,
                offset,
            });
            offset += l.size;
        }
        let size = align_up(offset, max_align).max(1);
        let st = &mut self.structs[id.0];
        st.fields = laid_out;
        st.size = size;
        st.align = max_align;
        let tid = self.intern(TypeKind::Struct(id));
        self.layouts[tid.0] = Layout {
            size,
            align: max_align,
        };
    }

    pub fn new_enum(&mut self, name: String, repr: TypeId, variants: HashMap<String, i64>) -> EnumId {
        let id = EnumId(self.enums.len());
        self.enums.push(EnumInfo {
            id,
            name,
            repr,
            variants,
        });
        self.intern(TypeKind::Enum(id));
        id
    }

    fn compute_layout(&self, kind: &TypeKind) -> Layout {
        match kind {
            TypeKind::Void => Layout { size: 0, align: 1 },
            TypeKind::Bool | TypeKind::I8 | TypeKind::U8 => Layout { size: 1, align: 1 },
            TypeKind::I16 | TypeKind::U16 => Layout { size: 2, align: 2 },
            TypeKind::I32 | TypeKind::U32 | TypeKind::F32 => Layout { size: 4, align: 4 },
            TypeKind::I64
            | TypeKind::U64
            | TypeKind::F64
            | TypeKind::OpaquePtr
            | TypeKind::CStr
            | TypeKind::Ptr(_)
            | TypeKind::Fn(_, _) => Layout { size: 8, align: 8 },
            TypeKind::Str | TypeKind::Slice(_) => Layout { size: 16, align: 8 },
            TypeKind::Array(elem, len) => {
                let l = self.layout(*elem);
                Layout {
                    size: l.size.saturating_mul(*len as u32),
                    align: l.align,
                }
            }
            TypeKind::Struct(sid) => {
                let st = &self.structs[sid.0];
                Layout {
                    size: st.size,
                    align: st.align,
                }
            }
            TypeKind::Enum(eid) => self.layout(self.enums[eid.0].repr),
        }
    }

    pub fn is_int(&self, id: TypeId) -> bool {
        matches!(
            self.kind(id),
            TypeKind::I8
                | TypeKind::I16
                | TypeKind::I32
                | TypeKind::I64
                | TypeKind::U8
                | TypeKind::U16
                | TypeKind::U32
                | TypeKind::U64
        )
    }

    pub fn is_signed_int(&self, id: TypeId) -> bool {
        matches!(
            self.kind(id),
            TypeKind::I8 | TypeKind::I16 | TypeKind::I32 | TypeKind::I64
        )
    }

    pub fn is_unsigned_int(&self, id: TypeId) -> bool {
        matches!(
            self.kind(id),
            TypeKind::U8 | TypeKind::U16 | TypeKind::U32 | TypeKind::U64
        )
    }

    pub fn is_float(&self, id: TypeId) -> bool {
        matches!(self.kind(id), TypeKind::F32 | TypeKind::F64)
    }

    pub fn is_numeric(&self, id: TypeId) -> bool {
        self.is_int(id) || self.is_float(id)
    }

    pub fn is_ptr_like(&self, id: TypeId) -> bool {
        matches!(
            self.kind(id),
            TypeKind::OpaquePtr | TypeKind::CStr | TypeKind::Ptr(_) | TypeKind::Fn(_, _)
        )
    }

    pub fn is_aggregate(&self, id: TypeId) -> bool {
        matches!(
            self.kind(id),
            TypeKind::Str | TypeKind::Slice(_) | TypeKind::Array(_, _) | TypeKind::Struct(_)
        )
    }

    pub fn int_bounds(&self, id: TypeId) -> Option<(i128, i128)> {
        match self.kind(id) {
            TypeKind::I8 => Some((i8::MIN as i128, i8::MAX as i128)),
            TypeKind::I16 => Some((i16::MIN as i128, i16::MAX as i128)),
            TypeKind::I32 => Some((i32::MIN as i128, i32::MAX as i128)),
            TypeKind::I64 => Some((i64::MIN as i128, i64::MAX as i128)),
            TypeKind::U8 => Some((0, u8::MAX as i128)),
            TypeKind::U16 => Some((0, u16::MAX as i128)),
            TypeKind::U32 => Some((0, u32::MAX as i128)),
            TypeKind::U64 => Some((0, u64::MAX as i128)),
            _ => None,
        }
    }

    pub fn display(&self, id: TypeId) -> String {
        match self.kind(id) {
            TypeKind::Void => "void".into(),
            TypeKind::Bool => "bool".into(),
            TypeKind::I8 => "i8".into(),
            TypeKind::I16 => "i16".into(),
            TypeKind::I32 => "i32".into(),
            TypeKind::I64 => "i64".into(),
            TypeKind::U8 => "u8".into(),
            TypeKind::U16 => "u16".into(),
            TypeKind::U32 => "u32".into(),
            TypeKind::U64 => "u64".into(),
            TypeKind::F32 => "f32".into(),
            TypeKind::F64 => "f64".into(),
            TypeKind::OpaquePtr => "ptr".into(),
            TypeKind::CStr => "cstr".into(),
            TypeKind::Str => "str".into(),
            TypeKind::Ptr(inner) => format!("*{}", self.display(*inner)),
            TypeKind::Array(inner, len) => format!("[{}; {}]", self.display(*inner), len),
            TypeKind::Slice(inner) => format!("[]{}", self.display(*inner)),
            TypeKind::Fn(params, ret) => {
                let ps: Vec<String> = params.iter().map(|p| self.display(*p)).collect();
                if *ret == TY_VOID {
                    format!("fn({})", ps.join(", "))
                } else {
                    format!("fn({}) -> {}", ps.join(", "), self.display(*ret))
                }
            }
            TypeKind::Struct(sid) => self.structs[sid.0].name.clone(),
            TypeKind::Enum(eid) => self.enums[eid.0].name.clone(),
        }
    }
}

pub fn align_up(val: u32, align: u32) -> u32 {
    if align <= 1 {
        val
    } else {
        (val + align - 1) & !(align - 1)
    }
}

impl fmt::Display for TypeId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Type({})", self.0)
    }
}
