use std::collections::{HashMap, HashSet};
use std::fs;
use std::path::{Path, PathBuf};

use crate::ast::{ImportDecl, ModuleAst};
use crate::error::{Diagnostic, SourceFile, Span};
use crate::parser::parse_module;

const STD_WIN32: &str = include_str!("../stdlib/win32.velo");
const STD_MATH: &str = include_str!("../stdlib/math.velo");

#[derive(Debug, Clone)]
pub struct ResolvedImport {
    pub target_mod: usize,
    pub bind_name: String,
    pub from_items: Option<Vec<String>>,
}

#[allow(dead_code)]
#[derive(Debug, Clone)]
pub struct LoadedModule {
    pub file_id: usize,
    pub name: String,
    pub ast: ModuleAst,
    pub imports: Vec<ResolvedImport>,
}

pub struct LoadedProject {
    pub files: Vec<SourceFile>,
    pub modules: Vec<LoadedModule>,
    pub entry_mod: usize,
}

pub fn load_project(entry_path: &Path) -> Result<LoadedProject, (Vec<SourceFile>, Diagnostic)> {
    let mut loader = Loader {
        files: Vec::new(),
        modules: Vec::new(),
        path_to_module: HashMap::new(),
        std_to_module: HashMap::new(),
        visiting: HashSet::new(),
    };

    let canon = fs::canonicalize(entry_path).unwrap_or_else(|_| entry_path.to_path_buf());
    let name = canon
        .file_stem()
        .and_then(|s| s.to_str())
        .unwrap_or("main")
        .to_string();

    match loader.load_file(&canon, name, None) {
        Ok(entry_mod) => Ok(LoadedProject {
            files: loader.files,
            modules: loader.modules,
            entry_mod,
        }),
        Err(d) => Err((loader.files, d)),
    }
}

struct Loader {
    files: Vec<SourceFile>,
    modules: Vec<LoadedModule>,
    path_to_module: HashMap<PathBuf, usize>,
    std_to_module: HashMap<String, usize>,
    visiting: HashSet<PathBuf>,
}

impl Loader {
    fn load_file(
        &mut self,
        path: &Path,
        module_key: String,
        import_span: Option<Span>,
    ) -> Result<usize, Diagnostic> {
        let canon = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if let Some(&idx) = self.path_to_module.get(&canon) {
            return Ok(idx);
        }
        if self.visiting.contains(&canon) {
            return Err(Diagnostic::error(
                import_span.unwrap_or_default(),
                format!("circular import involving '{}'", path.display()),
            ));
        }

        let source = fs::read_to_string(&canon).map_err(|e| {
            Diagnostic::error(
                import_span.unwrap_or_default(),
                format!("cannot read '{}': {e}", path.display()),
            )
        })?;

        self.visiting.insert(canon.clone());
        let file_id = self.files.len();
        self.files.push(SourceFile::new(canon.clone(), source.clone()));
        let ast = parse_module(file_id, &source)?;

        let base_dir = canon.parent().unwrap_or_else(|| Path::new(".")).to_path_buf();
        let mut resolved = Vec::new();
        for imp in &ast.imports {
            let target_mod = self.resolve_import(&base_dir, imp)?;
            let bind_name = imp.alias.clone().unwrap_or_else(|| {
                if imp.is_file {
                    Path::new(&imp.path)
                        .file_stem()
                        .and_then(|s| s.to_str())
                        .unwrap_or(&imp.path)
                        .to_string()
                } else {
                    imp.path
                        .rsplit('/')
                        .next()
                        .unwrap_or(&imp.path)
                        .to_string()
                }
            });
            resolved.push(ResolvedImport {
                target_mod,
                bind_name,
                from_items: imp.from_items.clone(),
            });
        }

        self.visiting.remove(&canon);
        let mod_idx = self.modules.len();
        self.modules.push(LoadedModule {
            file_id,
            name: module_key,
            ast,
            imports: resolved,
        });
        self.path_to_module.insert(canon, mod_idx);
        Ok(mod_idx)
    }

    fn load_builtin_or_stdlib(
        &mut self,
        name: &str,
        base_dir: &Path,
        span: Span,
    ) -> Result<usize, Diagnostic> {
        if let Some(&idx) = self.std_to_module.get(name) {
            return Ok(idx);
        }

        let mut candidates = vec![
            base_dir.join("stdlib").join(format!("{name}.velo")),
            base_dir.join("..").join("stdlib").join(format!("{name}.velo")),
            PathBuf::from("stdlib").join(format!("{name}.velo")),
        ];
        if let Ok(exe) = std::env::current_exe() {
            if let Some(dir) = exe.parent() {
                candidates.push(dir.join("stdlib").join(format!("{name}.velo")));
            }
        }
        for cand in candidates {
            if cand.exists() {
                let idx = self.load_file(&cand, name.to_string(), Some(span))?;
                self.std_to_module.insert(name.to_string(), idx);
                return Ok(idx);
            }
        }

        let embedded = match name {
            "win32" => Some(STD_WIN32),
            "math" => Some(STD_MATH),
            _ => None,
        };
        let Some(src) = embedded else {
            return Err(Diagnostic::error(
                span,
                format!("cannot find module '{name}'"),
            ));
        };

        let file_id = self.files.len();
        self.files.push(SourceFile::new(
            PathBuf::from(format!("<stdlib>/{name}.velo")),
            src.to_string(),
        ));
        let ast = parse_module(file_id, src)?;
        let mod_idx = self.modules.len();
        self.modules.push(LoadedModule {
            file_id,
            name: name.to_string(),
            ast,
            imports: Vec::new(),
        });
        self.std_to_module.insert(name.to_string(), mod_idx);
        Ok(mod_idx)
    }

    fn resolve_import(&mut self, base_dir: &Path, imp: &ImportDecl) -> Result<usize, Diagnostic> {
        if imp.is_file {
            let p = base_dir.join(&imp.path);
            let key = imp.alias.clone().unwrap_or_else(|| {
                Path::new(&imp.path)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or(&imp.path)
                    .to_string()
            });
            self.load_file(&p, key, Some(imp.span))
        } else {
            let rel = base_dir.join(format!("{}.velo", imp.path));
            if rel.exists() {
                self.load_file(&rel, imp.path.clone(), Some(imp.span))
            } else {
                self.load_builtin_or_stdlib(&imp.path, base_dir, imp.span)
            }
        }
    }
}
