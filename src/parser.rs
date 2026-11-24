use crate::ast::*;
use crate::error::{Diagnostic, Span};
use crate::lexer::{tokenize, FStrTokenPart, Token, TokenKind};

pub fn parse_module(file_id: usize, src: &str) -> Result<ModuleAst, Diagnostic> {
    let tokens = tokenize(file_id, src)?;
    let mut p = Parser {
        file_id,
        tokens,
        idx: 0,
    };
    p.parse_module_items()
}

struct Parser {
    file_id: usize,
    tokens: Vec<Token>,
    idx: usize,
}

impl Parser {
    fn peek(&self) -> &Token {
        &self.tokens[self.idx.min(self.tokens.len() - 1)]
    }

    fn peek_next(&self) -> &Token {
        &self.tokens[(self.idx + 1).min(self.tokens.len() - 1)]
    }

    fn at(&self, kind: &TokenKind) -> bool {
        &self.peek().kind == kind
    }

    fn advance(&mut self) -> Token {
        let tok = self.peek().clone();
        if self.idx + 1 < self.tokens.len() {
            self.idx += 1;
        }
        tok
    }

    fn skip_newlines(&mut self) {
        while matches!(self.peek().kind, TokenKind::Newline) {
            self.advance();
        }
    }

    fn expect(&mut self, kind: TokenKind, msg: &str) -> Result<Token, Diagnostic> {
        if self.peek().kind == kind {
            Ok(self.advance())
        } else {
            Err(Diagnostic::error(
                self.peek().span,
                format!("{msg}; found {:?}", self.peek().kind),
            ))
        }
    }

    fn expect_ident(&mut self, msg: &str) -> Result<(String, Span), Diagnostic> {
        match &self.peek().kind {
            TokenKind::Ident(s) => {
                let s = s.clone();
                let sp = self.advance().span;
                Ok((s, sp))
            }
            _ => Err(Diagnostic::error(
                self.peek().span,
                format!("{msg}; found {:?}", self.peek().kind),
            )),
        }
    }

    fn expect_field_ident(&mut self, msg: &str) -> Result<(String, Span), Diagnostic> {
        match &self.peek().kind {
            TokenKind::Ident(s) => {
                let s = s.clone();
                let sp = self.advance().span;
                Ok((s, sp))
            }
            TokenKind::PtrKw => {
                let sp = self.advance().span;
                Ok(("ptr".to_string(), sp))
            }
            _ => Err(Diagnostic::error(
                self.peek().span,
                format!("{msg}; found {:?}", self.peek().kind),
            )),
        }
    }

    fn expect_newline_or_eof(&mut self) -> Result<(), Diagnostic> {
        if matches!(
            self.peek().kind,
            TokenKind::Newline | TokenKind::Eof | TokenKind::Dedent
        ) {
            if self.at(&TokenKind::Newline) {
                self.advance();
            }
            Ok(())
        } else {
            Err(Diagnostic::error(
                self.peek().span,
                format!("expected end of line; found {:?}", self.peek().kind),
            ))
        }
    }

    fn parse_module_items(&mut self) -> Result<ModuleAst, Diagnostic> {
        let mut m = ModuleAst {
            file_id: self.file_id,
            imports: Vec::new(),
            links: Vec::new(),
            subsystem: None,
            consts: Vec::new(),
            enums: Vec::new(),
            structs: Vec::new(),
            externs: Vec::new(),
            funcs: Vec::new(),
        };

        self.skip_newlines();
        while !self.at(&TokenKind::Eof) {
            match &self.peek().kind {
                TokenKind::Import => m.imports.push(self.parse_import()?),
                TokenKind::From => m.imports.push(self.parse_from_import()?),
                TokenKind::Link => {
                    let sp = self.advance().span;
                    let lib = match &self.peek().kind {
                        TokenKind::Str(s) => {
                            let s = s.clone();
                            self.advance();
                            s
                        }
                        _ => {
                            return Err(Diagnostic::error(
                                self.peek().span,
                                "expected library name string after 'link'",
                            ))
                        }
                    };
                    self.expect_newline_or_eof()?;
                    m.links.push((lib, sp));
                }
                TokenKind::Subsystem => {
                    let sp = self.advance().span;
                    let sub = match &self.peek().kind {
                        TokenKind::Str(s) => {
                            let s = s.clone();
                            self.advance();
                            s
                        }
                        _ => {
                            return Err(Diagnostic::error(
                                self.peek().span,
                                "expected \"windows\" or \"console\" after 'subsystem'",
                            ))
                        }
                    };
                    self.expect_newline_or_eof()?;
                    m.subsystem = Some((sub, sp));
                }
                TokenKind::Const => m.consts.push(self.parse_const_decl()?),
                TokenKind::Enum => m.enums.push(self.parse_enum_decl()?),
                TokenKind::Struct => m.structs.push(self.parse_struct_decl()?),
                TokenKind::Extern => m.externs.push(self.parse_extern_decl()?),
                TokenKind::Inline => {
                    self.advance();
                    m.funcs.push(self.parse_fn_decl(true)?);
                }
                TokenKind::Def => m.funcs.push(self.parse_fn_decl(false)?),
                _ => {
                    return Err(Diagnostic::error(
                        self.peek().span,
                        format!("unexpected top-level token {:?}", self.peek().kind),
                    ))
                }
            }
            self.skip_newlines();
        }
        Ok(m)
    }

    fn parse_import(&mut self) -> Result<ImportDecl, Diagnostic> {
        let sp = self.advance().span; // import
        let (path, is_file) = match &self.peek().kind {
            TokenKind::Str(s) => {
                let s = s.clone();
                self.advance();
                (s, true)
            }
            TokenKind::Ident(s) => {
                let mut full = s.clone();
                self.advance();
                while self.at(&TokenKind::Dot) {
                    self.advance();
                    let (part, _) = self.expect_ident("expected module name after '.'")?;
                    full.push('/');
                    full.push_str(&part);
                }
                (full, false)
            }
            _ => {
                return Err(Diagnostic::error(
                    self.peek().span,
                    "expected module name or file path after 'import'",
                ))
            }
        };
        let alias = if self.at(&TokenKind::As) {
            self.advance();
            Some(self.expect_ident("expected alias after 'as'")?.0)
        } else {
            None
        };
        self.expect_newline_or_eof()?;
        Ok(ImportDecl {
            path,
            is_file,
            alias,
            from_items: None,
            span: sp,
        })
    }

    fn parse_from_import(&mut self) -> Result<ImportDecl, Diagnostic> {
        let sp = self.advance().span; // from
        let (path, is_file) = match &self.peek().kind {
            TokenKind::Str(s) => {
                let s = s.clone();
                self.advance();
                (s, true)
            }
            TokenKind::Ident(s) => {
                let mut full = s.clone();
                self.advance();
                while self.at(&TokenKind::Dot) {
                    self.advance();
                    let (part, _) = self.expect_ident("expected module name after '.'")?;
                    full.push('/');
                    full.push_str(&part);
                }
                (full, false)
            }
            _ => {
                return Err(Diagnostic::error(
                    self.peek().span,
                    "expected module name or path after 'from'",
                ))
            }
        };
        self.expect(TokenKind::Import, "expected 'import'")?;
        let mut items = Vec::new();
        if self.at(&TokenKind::Star) {
            self.advance();
            items.push("*".to_string());
        } else {
            loop {
                let (name, _) = self.expect_ident("expected symbol name in from-import")?;
                items.push(name);
                if self.at(&TokenKind::Comma) {
                    self.advance();
                } else {
                    break;
                }
            }
        }
        self.expect_newline_or_eof()?;
        Ok(ImportDecl {
            path,
            is_file,
            alias: None,
            from_items: Some(items),
            span: sp,
        })
    }

    fn parse_const_decl(&mut self) -> Result<ConstDecl, Diagnostic> {
        let sp = self.advance().span; // const
        let (name, _) = self.expect_ident("expected constant name")?;
        let ty = if self.at(&TokenKind::Colon) {
            self.advance();
            Some(self.parse_type()?)
        } else {
            None
        };
        self.expect(TokenKind::Eq, "expected '=' in const declaration")?;
        let value = self.parse_expr()?;
        self.expect_newline_or_eof()?;
        Ok(ConstDecl { name, ty, value, span: sp })
    }

    fn parse_enum_decl(&mut self) -> Result<EnumDecl, Diagnostic> {
        let sp = self.advance().span; // enum
        let (name, _) = self.expect_ident("expected enum name")?;
        let repr = if self.at(&TokenKind::LParen) {
            self.advance();
            let t = self.parse_type()?;
            self.expect(TokenKind::RParen, "expected ')' after enum base type")?;
            Some(t)
        } else {
            None
        };
        self.expect(TokenKind::Colon, "expected ':' after enum header")?;
        self.expect_newline_or_eof()?;
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented enum body")?;
        let mut variants = Vec::new();
        while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            let (vname, vsp) = self.expect_ident("expected enum variant name")?;
            let val = if self.at(&TokenKind::Eq) {
                self.advance();
                let neg = if self.at(&TokenKind::Minus) {
                    self.advance();
                    true
                } else {
                    false
                };
                match self.peek().kind {
                    TokenKind::Int(n) => {
                        self.advance();
                        let v = if neg { -(n as i64) } else { n as i64 };
                        Some(v)
                    }
                    _ => {
                        return Err(Diagnostic::error(
                            self.peek().span,
                            "expected integer constant for enum variant value",
                        ))
                    }
                }
            } else {
                None
            };
            self.expect_newline_or_eof()?;
            variants.push(EnumVariant {
                name: vname,
                value: val,
                span: vsp,
            });
            self.skip_newlines();
        }
        self.expect(TokenKind::Dedent, "expected end of enum block")?;
        Ok(EnumDecl {
            name,
            repr,
            variants,
            span: sp,
        })
    }

    fn parse_struct_decl(&mut self) -> Result<StructDecl, Diagnostic> {
        let sp = self.advance().span; // struct
        let (name, _) = self.expect_ident("expected struct name")?;
        self.expect(TokenKind::Colon, "expected ':' after struct name")?;
        self.expect_newline_or_eof()?;
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented struct body")?;

        let mut fields = Vec::new();
        let mut methods = Vec::new();
        while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            match &self.peek().kind {
                TokenKind::Pass => {
                    self.advance();
                    self.expect_newline_or_eof()?;
                }
                TokenKind::Inline => {
                    self.advance();
                    methods.push(self.parse_fn_decl(true)?);
                }
                TokenKind::Def => {
                    methods.push(self.parse_fn_decl(false)?);
                }
                TokenKind::Ident(_) => {
                    let (fname, fsp) = self.expect_ident("expected field name")?;
                    self.expect(TokenKind::Colon, "expected ':' after field name")?;
                    let fty = self.parse_type()?;
                    self.expect_newline_or_eof()?;
                    fields.push(FieldDecl {
                        name: fname,
                        ty: fty,
                        span: fsp,
                    });
                }
                _ => {
                    return Err(Diagnostic::error(
                        self.peek().span,
                        "expected struct field or method definition",
                    ))
                }
            }
            self.skip_newlines();
        }
        self.expect(TokenKind::Dedent, "expected end of struct block")?;
        Ok(StructDecl {
            name,
            fields,
            methods,
            span: sp,
        })
    }

    fn parse_extern_decl(&mut self) -> Result<ExternDecl, Diagnostic> {
        let sp = self.advance().span; // extern
        self.expect(TokenKind::Def, "expected 'def' after 'extern'")?;
        let (name, _) = self.expect_ident("expected extern function name")?;
        self.expect(TokenKind::LParen, "expected '(' after extern function name")?;
        let (params, variadic) = self.parse_param_list(true)?;
        let ret = if self.at(&TokenKind::Arrow) {
            self.advance();
            self.parse_type()?
        } else {
            TypeExpr::Void(sp)
        };
        self.expect_newline_or_eof()?;
        Ok(ExternDecl {
            name,
            params,
            ret,
            variadic,
            span: sp,
        })
    }

    fn parse_fn_decl(&mut self, inline_hint: bool) -> Result<FnDecl, Diagnostic> {
        let sp = self.advance().span; // def
        let (name, _) = self.expect_ident("expected function name")?;
        self.expect(TokenKind::LParen, "expected '(' after function name")?;
        let (params, _) = self.parse_param_list(false)?;
        let ret = if self.at(&TokenKind::Arrow) {
            self.advance();
            self.parse_type()?
        } else {
            TypeExpr::Void(sp)
        };
        self.expect(TokenKind::Colon, "expected ':' before function body")?;
        let body = self.parse_block()?;
        Ok(FnDecl {
            name,
            params,
            ret,
            inline_hint,
            body,
            span: sp,
        })
    }

    fn parse_param_list(&mut self, allow_variadic: bool) -> Result<(Vec<ParamDecl>, bool), Diagnostic> {
        let mut params = Vec::new();
        let mut variadic = false;
        while !self.at(&TokenKind::RParen) {
            if allow_variadic && self.at(&TokenKind::DotDotDot) {
                self.advance();
                variadic = true;
                if self.at(&TokenKind::Comma) {
                    self.advance();
                }
                break;
            }
            let psp = self.peek().span;
            let is_var = if self.at(&TokenKind::Var) {
                self.advance();
                true
            } else {
                false
            };
            let (pname, _) = self.expect_ident("expected parameter name")?;
            let is_self = pname == "self";
            let ty = if self.at(&TokenKind::Colon) {
                self.advance();
                Some(self.parse_type()?)
            } else if is_self {
                None
            } else {
                return Err(Diagnostic::error(
                    self.peek().span,
                    format!("parameter '{pname}' requires a type annotation"),
                ));
            };
            params.push(ParamDecl {
                name: pname,
                is_var,
                is_self,
                ty,
                span: psp,
            });
            if self.at(&TokenKind::Comma) {
                self.advance();
            } else {
                break;
            }
        }
        self.expect(TokenKind::RParen, "expected ')' after parameter list")?;
        Ok((params, variadic))
    }

    fn parse_block(&mut self) -> Result<Vec<Stmt>, Diagnostic> {
        self.expect_newline_or_eof()?;
        self.skip_newlines();
        self.expect(TokenKind::Indent, "expected indented block")?;
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
            self.skip_newlines();
            if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                break;
            }
            stmts.push(self.parse_stmt()?);
            self.skip_newlines();
        }
        self.expect(TokenKind::Dedent, "expected end of indented block")?;
        Ok(stmts)
    }

    fn parse_stmt(&mut self) -> Result<Stmt, Diagnostic> {
        let sp = self.peek().span;
        match &self.peek().kind {
            TokenKind::Let => {
                self.advance();
                let (name, _) = self.expect_ident("expected variable name after 'let'")?;
                let ty = if self.at(&TokenKind::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(TokenKind::Eq, "expected '=' in let binding")?;
                let value = self.parse_expr()?;
                self.expect_newline_or_eof()?;
                Ok(Stmt {
                    kind: StmtKind::Let { name, ty, value },
                    span: sp,
                })
            }
            TokenKind::Var => {
                self.advance();
                let (name, _) = self.expect_ident("expected variable name after 'var'")?;
                let ty = if self.at(&TokenKind::Colon) {
                    self.advance();
                    Some(self.parse_type()?)
                } else {
                    None
                };
                self.expect(TokenKind::Eq, "expected '=' in var binding")?;
                let value = self.parse_expr()?;
                self.expect_newline_or_eof()?;
                Ok(Stmt {
                    kind: StmtKind::Var { name, ty, value },
                    span: sp,
                })
            }
            TokenKind::Const => {
                let c = self.parse_const_decl()?;
                Ok(Stmt {
                    kind: StmtKind::Const(c),
                    span: sp,
                })
            }
            TokenKind::If => {
                self.advance();
                let cond = self.parse_expr()?;
                self.expect(TokenKind::Colon, "expected ':' after if condition")?;
                let body = self.parse_block()?;
                let mut branches = vec![(cond, body)];
                self.skip_newlines();
                while self.at(&TokenKind::Elif) {
                    self.advance();
                    let econd = self.parse_expr()?;
                    self.expect(TokenKind::Colon, "expected ':' after elif condition")?;
                    let ebody = self.parse_block()?;
                    branches.push((econd, ebody));
                    self.skip_newlines();
                }
                let else_body = if self.at(&TokenKind::Else) {
                    self.advance();
                    self.expect(TokenKind::Colon, "expected ':' after else")?;
                    Some(self.parse_block()?)
                } else {
                    None
                };
                Ok(Stmt {
                    kind: StmtKind::If { branches, else_body },
                    span: sp,
                })
            }
            TokenKind::While => {
                self.advance();
                let cond = self.parse_expr()?;
                self.expect(TokenKind::Colon, "expected ':' after while condition")?;
                let body = self.parse_block()?;
                Ok(Stmt {
                    kind: StmtKind::While { cond, body },
                    span: sp,
                })
            }
            TokenKind::For => {
                self.advance();
                let (var_name, _) = self.expect_ident("expected loop variable after 'for'")?;
                self.expect(TokenKind::In, "expected 'in' after loop variable")?;
                let first = self.parse_expr()?;
                if self.at(&TokenKind::DotDot) || self.at(&TokenKind::DotDotEq) {
                    let inclusive = self.at(&TokenKind::DotDotEq);
                    self.advance();
                    let end = self.parse_expr()?;
                    self.expect(TokenKind::Colon, "expected ':' after range")?;
                    let body = self.parse_block()?;
                    Ok(Stmt {
                        kind: StmtKind::ForRange {
                            var_name,
                            start: first,
                            end,
                            inclusive,
                            body,
                        },
                        span: sp,
                    })
                } else {
                    self.expect(TokenKind::Colon, "expected ':' after for iterable")?;
                    let body = self.parse_block()?;
                    Ok(Stmt {
                        kind: StmtKind::ForIter {
                            var_name,
                            iter: first,
                            body,
                        },
                        span: sp,
                    })
                }
            }
            TokenKind::Match => {
                self.advance();
                let expr = self.parse_expr()?;
                self.expect(TokenKind::Colon, "expected ':' after match expression")?;
                self.expect_newline_or_eof()?;
                self.skip_newlines();
                self.expect(TokenKind::Indent, "expected indented match cases")?;
                let mut cases = Vec::new();
                let mut else_body = None;
                while !self.at(&TokenKind::Dedent) && !self.at(&TokenKind::Eof) {
                    self.skip_newlines();
                    if self.at(&TokenKind::Dedent) || self.at(&TokenKind::Eof) {
                        break;
                    }
                    if self.at(&TokenKind::Case) {
                        self.advance();
                        let mut pats = Vec::new();
                        loop {
                            pats.push(self.parse_expr()?);
                            if self.at(&TokenKind::Comma) {
                                self.advance();
                            } else {
                                break;
                            }
                        }
                        self.expect(TokenKind::Colon, "expected ':' after case patterns")?;
                        let body = self.parse_block()?;
                        cases.push((pats, body));
                    } else if self.at(&TokenKind::Else) {
                        self.advance();
                        self.expect(TokenKind::Colon, "expected ':' after else")?;
                        else_body = Some(self.parse_block()?);
                    } else {
                        return Err(Diagnostic::error(
                            self.peek().span,
                            "expected 'case' or 'else' inside match",
                        ));
                    }
                    self.skip_newlines();
                }
                self.expect(TokenKind::Dedent, "expected end of match block")?;
                Ok(Stmt {
                    kind: StmtKind::Match {
                        expr,
                        cases,
                        else_body,
                    },
                    span: sp,
                })
            }
            TokenKind::Return => {
                self.advance();
                let val = if matches!(
                    self.peek().kind,
                    TokenKind::Newline | TokenKind::Eof | TokenKind::Dedent
                ) {
                    None
                } else {
                    Some(self.parse_expr()?)
                };
                self.expect_newline_or_eof()?;
                Ok(Stmt {
                    kind: StmtKind::Return(val),
                    span: sp,
                })
            }
            TokenKind::Break => {
                self.advance();
                self.expect_newline_or_eof()?;
                Ok(Stmt {
                    kind: StmtKind::Break,
                    span: sp,
                })
            }
            TokenKind::Continue => {
                self.advance();
                self.expect_newline_or_eof()?;
                Ok(Stmt {
                    kind: StmtKind::Continue,
                    span: sp,
                })
            }
            TokenKind::Pass => {
                self.advance();
                self.expect_newline_or_eof()?;
                Ok(Stmt {
                    kind: StmtKind::Pass,
                    span: sp,
                })
            }
            TokenKind::Defer => {
                self.advance();
                let inner = self.parse_stmt()?;
                Ok(Stmt {
                    kind: StmtKind::Defer(Box::new(inner)),
                    span: sp,
                })
            }
            _ => {
                let lhs = self.parse_expr()?;
                let assign_op = match self.peek().kind {
                    TokenKind::Eq => Some(None),
                    TokenKind::PlusEq => Some(Some(BinOp::Add)),
                    TokenKind::MinusEq => Some(Some(BinOp::Sub)),
                    TokenKind::StarEq => Some(Some(BinOp::Mul)),
                    TokenKind::SlashEq => Some(Some(BinOp::Div)),
                    TokenKind::PercentEq => Some(Some(BinOp::Mod)),
                    TokenKind::AmpEq => Some(Some(BinOp::BitAnd)),
                    TokenKind::PipeEq => Some(Some(BinOp::BitOr)),
                    TokenKind::CaretEq => Some(Some(BinOp::BitXor)),
                    TokenKind::ShlEq => Some(Some(BinOp::Shl)),
                    TokenKind::ShrEq => Some(Some(BinOp::Shr)),
                    _ => None,
                };
                if let Some(op) = assign_op {
                    self.advance();
                    let rhs = self.parse_expr()?;
                    self.expect_newline_or_eof()?;
                    Ok(Stmt {
                        kind: StmtKind::Assign {
                            target: lhs,
                            op,
                            value: rhs,
                        },
                        span: sp,
                    })
                } else {
                    self.expect_newline_or_eof()?;
                    Ok(Stmt {
                        kind: StmtKind::Expr(lhs),
                        span: sp,
                    })
                }
            }
        }
    }

    fn parse_type(&mut self) -> Result<TypeExpr, Diagnostic> {
        let sp = self.peek().span;
        match &self.peek().kind {
            TokenKind::PtrKw => {
                self.advance();
                Ok(TypeExpr::OpaquePtr(sp))
            }
            TokenKind::Star => {
                self.advance();
                let inner = self.parse_type()?;
                let end_sp = inner.span();
                Ok(TypeExpr::Ptr(Box::new(inner), sp.merge(end_sp)))
            }
            TokenKind::LBracket => {
                self.advance();
                if self.at(&TokenKind::RBracket) {
                    self.advance();
                    let elem = self.parse_type()?;
                    let end_sp = elem.span();
                    Ok(TypeExpr::Slice(Box::new(elem), sp.merge(end_sp)))
                } else {
                    let elem = self.parse_type()?;
                    self.expect(TokenKind::Semi, "expected ';' in fixed array type [T; N]")?;
                    let len = match self.peek().kind {
                        TokenKind::Int(n) if n >= 0 => {
                            self.advance();
                            n as u64
                        }
                        _ => {
                            return Err(Diagnostic::error(
                                self.peek().span,
                                "expected non-negative integer constant for array length",
                            ))
                        }
                    };
                    let rbr = self.expect(TokenKind::RBracket, "expected ']' after array length")?;
                    Ok(TypeExpr::Array(Box::new(elem), len, sp.merge(rbr.span)))
                }
            }
            TokenKind::Fn => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '(' after 'fn'")?;
                let mut params = Vec::new();
                while !self.at(&TokenKind::RParen) {
                    params.push(self.parse_type()?);
                    if self.at(&TokenKind::Comma) {
                        self.advance();
                    } else {
                        break;
                    }
                }
                let rp = self.expect(TokenKind::RParen, "expected ')' in fn type")?;
                let ret = if self.at(&TokenKind::Arrow) {
                    self.advance();
                    self.parse_type()?
                } else {
                    TypeExpr::Void(rp.span)
                };
                let end_sp = ret.span();
                Ok(TypeExpr::Fn(params, Box::new(ret), sp.merge(end_sp)))
            }
            TokenKind::Ident(name) => {
                let mut full = name.clone();
                let mut end_sp = self.advance().span;
                while self.at(&TokenKind::Dot) {
                    self.advance();
                    let (part, psp) = self.expect_ident("expected type name after '.'")?;
                    full.push('.');
                    full.push_str(&part);
                    end_sp = psp;
                }
                if full == "void" {
                    Ok(TypeExpr::Void(sp))
                } else {
                    Ok(TypeExpr::Named(full, sp.merge(end_sp)))
                }
            }
            _ => Err(Diagnostic::error(
                sp,
                format!("expected type; found {:?}", self.peek().kind),
            )),
        }
    }

    fn parse_expr(&mut self) -> Result<Expr, Diagnostic> {
        let expr = self.parse_pratt(1)?;
        if self.at(&TokenKind::If) {
            self.advance();
            let cond = self.parse_pratt(1)?;
            self.expect(TokenKind::Else, "expected 'else' in conditional expression")?;
            let else_expr = self.parse_expr()?;
            let sp = expr.span.merge(else_expr.span);
            Ok(Expr {
                kind: ExprKind::Ternary {
                    then_expr: Box::new(expr),
                    cond: Box::new(cond),
                    else_expr: Box::new(else_expr),
                },
                span: sp,
            })
        } else {
            Ok(expr)
        }
    }

    fn parse_pratt(&mut self, min_prec: u8) -> Result<Expr, Diagnostic> {
        let mut lhs = self.parse_prefix()?;
        loop {
            if self.at(&TokenKind::As) && 11 >= min_prec {
                self.advance();
                let ty = self.parse_type()?;
                let sp = lhs.span.merge(ty.span());
                lhs = Expr {
                    kind: ExprKind::Cast {
                        expr: Box::new(lhs),
                        ty,
                    },
                    span: sp,
                };
                continue;
            }
            let Some((prec, op)) = self.peek_binop() else {
                break;
            };
            if prec < min_prec {
                break;
            }
            self.advance();
            let rhs = self.parse_pratt(prec + 1)?;
            let sp = lhs.span.merge(rhs.span);
            lhs = Expr {
                kind: ExprKind::Binary {
                    op,
                    left: Box::new(lhs),
                    right: Box::new(rhs),
                },
                span: sp,
            };
        }
        Ok(lhs)
    }

    fn peek_binop(&self) -> Option<(u8, BinOp)> {
        match self.peek().kind {
            TokenKind::Or => Some((1, BinOp::Or)),
            TokenKind::And => Some((2, BinOp::And)),
            TokenKind::EqEq => Some((3, BinOp::Eq)),
            TokenKind::BangEq => Some((3, BinOp::Ne)),
            TokenKind::Lt => Some((4, BinOp::Lt)),
            TokenKind::LtEq => Some((4, BinOp::Le)),
            TokenKind::Gt => Some((4, BinOp::Gt)),
            TokenKind::GtEq => Some((4, BinOp::Ge)),
            TokenKind::Pipe => Some((5, BinOp::BitOr)),
            TokenKind::Caret => Some((6, BinOp::BitXor)),
            TokenKind::Amp => Some((7, BinOp::BitAnd)),
            TokenKind::Shl => Some((8, BinOp::Shl)),
            TokenKind::Shr => Some((8, BinOp::Shr)),
            TokenKind::Plus => Some((9, BinOp::Add)),
            TokenKind::Minus => Some((9, BinOp::Sub)),
            TokenKind::Star => Some((10, BinOp::Mul)),
            TokenKind::Slash => Some((10, BinOp::Div)),
            TokenKind::Percent => Some((10, BinOp::Mod)),
            _ => None,
        }
    }

    fn parse_prefix(&mut self) -> Result<Expr, Diagnostic> {
        let sp = self.peek().span;
        let expr = match &self.peek().kind {
            TokenKind::Minus => {
                self.advance();
                let e = self.parse_prefix()?;
                let end = e.span;
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Neg,
                        expr: Box::new(e),
                    },
                    span: sp.merge(end),
                }
            }
            TokenKind::Plus => {
                self.advance();
                self.parse_prefix()?
            }
            TokenKind::Not => {
                self.advance();
                let e = self.parse_pratt(3)?;
                let end = e.span;
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::Not,
                        expr: Box::new(e),
                    },
                    span: sp.merge(end),
                }
            }
            TokenKind::Tilde => {
                self.advance();
                let e = self.parse_prefix()?;
                let end = e.span;
                Expr {
                    kind: ExprKind::Unary {
                        op: UnaryOp::BitNot,
                        expr: Box::new(e),
                    },
                    span: sp.merge(end),
                }
            }
            TokenKind::Amp => {
                self.advance();
                let e = self.parse_prefix()?;
                let end = e.span;
                Expr {
                    kind: ExprKind::AddrOf(Box::new(e)),
                    span: sp.merge(end),
                }
            }
            TokenKind::Star => {
                self.advance();
                let e = self.parse_prefix()?;
                let end = e.span;
                Expr {
                    kind: ExprKind::Deref(Box::new(e)),
                    span: sp.merge(end),
                }
            }
            _ => return self.parse_postfix(),
        };
        Ok(expr)
    }

    fn parse_postfix(&mut self) -> Result<Expr, Diagnostic> {
        let mut expr = self.parse_primary()?;
        loop {
            match &self.peek().kind {
                TokenKind::LParen => {
                    self.advance();
                    // Check if named-field struct literal: `Ident(field: val, ...)` or `mod.Ident(field: val, ...)`
                    if self.is_struct_lit_start() {
                        if let Some(name) = Self::extract_type_path(&expr) {
                            let mut fields = Vec::new();
                            while !self.at(&TokenKind::RParen) {
                                let (fname, _) = self.expect_ident("expected field name")?;
                                self.expect(TokenKind::Colon, "expected ':' after field name")?;
                                let fval = self.parse_expr()?;
                                fields.push((fname, fval));
                                if self.at(&TokenKind::Comma) {
                                    self.advance();
                                } else {
                                    break;
                                }
                            }
                            let rp = self.expect(TokenKind::RParen, "expected ')' after struct literal")?;
                            expr = Expr {
                                kind: ExprKind::StructLit { name, fields },
                                span: expr.span.merge(rp.span),
                            };
                            continue;
                        }
                    }
                    let mut args = Vec::new();
                    while !self.at(&TokenKind::RParen) {
                        args.push(self.parse_expr()?);
                        if self.at(&TokenKind::Comma) {
                            self.advance();
                        } else {
                            break;
                        }
                    }
                    let rp = self.expect(TokenKind::RParen, "expected ')' after arguments")?;
                    let sp = expr.span.merge(rp.span);
                    expr = Expr {
                        kind: ExprKind::Call {
                            callee: Box::new(expr),
                            args,
                        },
                        span: sp,
                    };
                }
                TokenKind::Dot => {
                    self.advance();
                    let (field, fsp) = self.expect_field_ident("expected field or method name after '.'")?;
                    let sp = expr.span.merge(fsp);
                    expr = Expr {
                        kind: ExprKind::Field {
                            obj: Box::new(expr),
                            field,
                        },
                        span: sp,
                    };
                }
                TokenKind::LBracket => {
                    self.advance();
                    if self.at(&TokenKind::DotDot) {
                        self.advance();
                        let end = if self.at(&TokenKind::RBracket) {
                            None
                        } else {
                            Some(Box::new(self.parse_expr()?))
                        };
                        let rb = self.expect(TokenKind::RBracket, "expected ']'")?;
                        let sp = expr.span.merge(rb.span);
                        expr = Expr {
                            kind: ExprKind::Slice {
                                obj: Box::new(expr),
                                start: None,
                                end,
                            },
                            span: sp,
                        };
                    } else {
                        let first = self.parse_expr()?;
                        if self.at(&TokenKind::DotDot) {
                            self.advance();
                            let end = if self.at(&TokenKind::RBracket) {
                                None
                            } else {
                                Some(Box::new(self.parse_expr()?))
                            };
                            let rb = self.expect(TokenKind::RBracket, "expected ']'")?;
                            let sp = expr.span.merge(rb.span);
                            expr = Expr {
                                kind: ExprKind::Slice {
                                    obj: Box::new(expr),
                                    start: Some(Box::new(first)),
                                    end,
                                },
                                span: sp,
                            };
                        } else {
                            let rb = self.expect(TokenKind::RBracket, "expected ']'")?;
                            let sp = expr.span.merge(rb.span);
                            expr = Expr {
                                kind: ExprKind::Index {
                                    obj: Box::new(expr),
                                    index: Box::new(first),
                                },
                                span: sp,
                            };
                        }
                    }
                }
                _ => break,
            }
        }
        Ok(expr)
    }

    fn is_struct_lit_start(&self) -> bool {
        matches!(self.peek().kind, TokenKind::Ident(_)) && self.peek_next().kind == TokenKind::Colon
    }

    fn extract_type_path(expr: &Expr) -> Option<String> {
        match &expr.kind {
            ExprKind::Ident(s) => Some(s.clone()),
            ExprKind::Field { obj, field } => {
                let mut prefix = Self::extract_type_path(obj)?;
                prefix.push('.');
                prefix.push_str(field);
                Some(prefix)
            }
            _ => None,
        }
    }

    fn parse_primary(&mut self) -> Result<Expr, Diagnostic> {
        let tok = self.peek().clone();
        match tok.kind {
            TokenKind::Int(v) => {
                self.advance();
                Ok(Expr { kind: ExprKind::Int(v), span: tok.span })
            }
            TokenKind::Float(v) => {
                self.advance();
                Ok(Expr { kind: ExprKind::Float(v), span: tok.span })
            }
            TokenKind::True => {
                self.advance();
                Ok(Expr { kind: ExprKind::Bool(true), span: tok.span })
            }
            TokenKind::False => {
                self.advance();
                Ok(Expr { kind: ExprKind::Bool(false), span: tok.span })
            }
            TokenKind::Null => {
                self.advance();
                Ok(Expr { kind: ExprKind::Null, span: tok.span })
            }
            TokenKind::Str(s) => {
                self.advance();
                Ok(Expr { kind: ExprKind::Str(s), span: tok.span })
            }
            TokenKind::CStr(s) => {
                self.advance();
                Ok(Expr { kind: ExprKind::CStr(s), span: tok.span })
            }
            TokenKind::WStr(s) => {
                self.advance();
                Ok(Expr { kind: ExprKind::WStr(s), span: tok.span })
            }
            TokenKind::FStr(parts) => {
                self.advance();
                let mut segs = Vec::new();
                for p in parts {
                    match p {
                        FStrTokenPart::Literal(l) => segs.push(FStrSeg::Lit(l)),
                        FStrTokenPart::Expr { src, prec, col_offset } => {
                            let sub_tokens = tokenize(self.file_id, &src).map_err(|mut d| {
                                d.span.line = tok.span.line;
                                d.span.col += col_offset.saturating_sub(1);
                                d
                            })?;
                            let mut sub_parser = Parser {
                                file_id: self.file_id,
                                tokens: sub_tokens,
                                idx: 0,
                            };
                            let mut sub_expr = sub_parser.parse_expr().map_err(|mut d| {
                                d.span.line = tok.span.line;
                                d.span.col += col_offset.saturating_sub(1);
                                d
                            })?;
                            sub_expr.span = Span::new(self.file_id, tok.span.line, col_offset, src.len().max(1));
                            segs.push(FStrSeg::Interp { expr: sub_expr, prec });
                        }
                    }
                }
                Ok(Expr { kind: ExprKind::FStr(segs), span: tok.span })
            }
            TokenKind::Ident(s) => {
                self.advance();
                Ok(Expr { kind: ExprKind::Ident(s), span: tok.span })
            }
            TokenKind::SizeOf => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '(' after 'sizeof'")?;
                let ty = self.parse_type()?;
                let rp = self.expect(TokenKind::RParen, "expected ')' after type in sizeof")?;
                Ok(Expr {
                    kind: ExprKind::SizeOf(ty),
                    span: tok.span.merge(rp.span),
                })
            }
            TokenKind::Zeroed => {
                self.advance();
                self.expect(TokenKind::LParen, "expected '(' after 'zeroed'")?;
                let ty = self.parse_type()?;
                let rp = self.expect(TokenKind::RParen, "expected ')' after type in zeroed")?;
                Ok(Expr {
                    kind: ExprKind::Zeroed(ty),
                    span: tok.span.merge(rp.span),
                })
            }
            TokenKind::LParen => {
                self.advance();
                let inner = self.parse_expr()?;
                self.expect(TokenKind::RParen, "expected ')'")?;
                Ok(inner)
            }
            TokenKind::LBracket => {
                self.advance();
                if self.at(&TokenKind::RBracket) {
                    let rb = self.advance();
                    return Ok(Expr {
                        kind: ExprKind::ArrayLit(Vec::new()),
                        span: tok.span.merge(rb.span),
                    });
                }
                let first = self.parse_expr()?;
                if self.at(&TokenKind::Semi) {
                    self.advance();
                    let count = match self.peek().kind {
                        TokenKind::Int(n) if n >= 0 => {
                            self.advance();
                            n as u64
                        }
                        _ => {
                            return Err(Diagnostic::error(
                                self.peek().span,
                                "expected non-negative integer constant in array repeat [val; N]",
                            ))
                        }
                    };
                    let rb = self.expect(TokenKind::RBracket, "expected ']'")?;
                    Ok(Expr {
                        kind: ExprKind::ArrayRepeat {
                            value: Box::new(first),
                            count,
                        },
                        span: tok.span.merge(rb.span),
                    })
                } else {
                    let mut elems = vec![first];
                    while self.at(&TokenKind::Comma) {
                        self.advance();
                        if self.at(&TokenKind::RBracket) {
                            break;
                        }
                        elems.push(self.parse_expr()?);
                    }
                    let rb = self.expect(TokenKind::RBracket, "expected ']'")?;
                    Ok(Expr {
                        kind: ExprKind::ArrayLit(elems),
                        span: tok.span.merge(rb.span),
                    })
                }
            }
            _ => Err(Diagnostic::error(
                tok.span,
                format!("expected expression; found {:?}", tok.kind),
            )),
        }
    }
}
