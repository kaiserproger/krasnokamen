//! Structural facts about `mc-core` production code, read with `syn`.
//!
//! The walk follows `mod` declarations (`#[path]` included) from `lib.rs`, so
//! only compiled files count. Test code is skipped: items under `#[cfg(test)]`
//! or `#[cfg(all(test, ..))]`, `#[test]` functions, and module files named
//! `*_tests.rs` or `tests.rs`. Comments never reach `syn`, and attributes (doc
//! comments included) are not visited.
//!
//! `syn` does not look inside macro invocations. A macro body that parses as
//! comma-separated expressions (`format!`, `matches!`, `assert!`, ...) is
//! visited like any other code. Any other body (`macro_rules!` templates,
//! `vec![x; n]`, `tracing` fields) is read token by token: string literals,
//! `panic!`, `crate::`/`super::` paths and the call shapes in `macro_tokens`.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use proc_macro2::{Delimiter, Group, Span, TokenStream, TokenTree};
use syn::punctuated::Punctuated;
use syn::spanned::Spanned;
use syn::visit::{self, Visit};
use syn::{
    Arm, Attribute, BinOp, Expr, ExprBinary, ExprMethodCall, Field, ImplItem, Item, ItemMod,
    ItemUse, Lit, LitStr, Local, Macro, Member, Meta, Stmt, Token, TraitItem, UseTree, Variant,
    Visibility,
};

/// The anti-pattern counters, in report order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Counter {
    /// String literal whose value starts with `minecraft:`.
    VanillaIdLiteral,
    /// `==`/`!=` with a `.path()` call on either side.
    PathEquality,
    /// `panic!` invocation.
    Panic,
    /// `.push(..)` on a field or variable named `merge_block_writes`.
    MergeBlockWritePush,
}

impl Counter {
    pub(super) const ALL: [Counter; 4] = [
        Counter::VanillaIdLiteral,
        Counter::PathEquality,
        Counter::Panic,
        Counter::MergeBlockWritePush,
    ];
}

#[derive(Debug, Default)]
pub(super) struct ModuleFacts {
    pub(super) name: String,
    /// Production lines of the files this module owns, `#[cfg(test)]` items
    /// excluded.
    pub(super) lines: usize,
    /// Referenced top-level module index → number of references.
    pub(super) refs: BTreeMap<usize, usize>,
    counts: [usize; Counter::ALL.len()],
}

impl ModuleFacts {
    pub(super) fn count(&self, counter: Counter) -> usize {
        self.counts[counter as usize]
    }
}

pub(super) struct CoreFacts {
    /// The children of `lib.rs`, sorted by name; index = graph node.
    pub(super) modules: Vec<ModuleFacts>,
    /// `lib.rs` itself: part of the anti-pattern totals, not a graph node.
    pub(super) root: ModuleFacts,
}

impl CoreFacts {
    pub(super) fn edges(&self) -> Vec<Vec<usize>> {
        self.modules
            .iter()
            .map(|module| module.refs.keys().copied().collect())
            .collect()
    }

    pub(super) fn total(&self, counter: Counter) -> usize {
        self.modules
            .iter()
            .chain([&self.root])
            .map(|module| module.count(counter))
            .sum()
    }
}

/// Reads the crate whose root is `src/lib.rs`.
pub(super) fn collect(src: &Path) -> Result<CoreFacts, String> {
    let lib = src.join("lib.rs");
    let (file, lines) = parse(&lib)?;
    let mut modules = BTreeMap::new();
    let mut root_uses = Vec::new();
    for item in &file.items {
        match item {
            Item::Mod(module) if !is_test_only(&module.attrs) => {
                // A missing file is reported by the walk below.
                let test_file = module.content.is_none()
                    && module_file(&lib, src, false, module)
                        .is_ok_and(|(path, _)| is_test_file(&path));
                if !test_file {
                    modules.insert(module.ident.to_string(), 0);
                }
            }
            Item::Use(item_use) if !is_test_only(&item_use.attrs) => root_uses.push(item_use),
            _ => {}
        }
    }
    for (index, slot) in modules.values_mut().enumerate() {
        *slot = index;
    }
    let mut aliases = BTreeMap::new();
    for item_use in root_uses {
        root_aliases(&modules, &item_use.tree, None, &mut aliases);
    }
    let resolver = Resolver { modules, aliases };

    let mut facts: Vec<ModuleFacts> = resolver
        .modules
        .keys()
        .map(|name| ModuleFacts {
            name: name.clone(),
            ..ModuleFacts::default()
        })
        .collect();
    facts.push(ModuleFacts {
        name: "lib.rs".into(),
        ..ModuleFacts::default()
    });
    let mut errors = Vec::new();
    let mut collector = Collector {
        resolver: &resolver,
        current: facts.len() - 1,
        facts: &mut facts,
        scope: Vec::new(),
        inline_depth: 0,
        file: PathBuf::new(),
        child_dir: PathBuf::new(),
        test_lines: Vec::new(),
        errors: &mut errors,
    };
    collector.walk_file(&file, lines, &lib, src.to_path_buf());
    if !errors.is_empty() {
        return Err(errors.join("; "));
    }
    let root = facts.pop().unwrap_or_default();
    Ok(CoreFacts {
        modules: facts,
        root,
    })
}

fn parse(path: &Path) -> Result<(syn::File, usize), String> {
    let source = fs::read_to_string(path)
        .map_err(|err| format!("{}: read failed: {err}", path.display()))?;
    let file = syn::parse_file(&source)
        .map_err(|err| format!("{}: parse failed: {err}", path.display()))?;
    Ok((file, source.lines().count()))
}

fn is_test_file(path: &Path) -> bool {
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name.ends_with("_tests.rs") || name == "tests.rs")
}

/// `#[cfg(test)]`, `#[cfg(all(.., test, ..))]` or `#[test]`.
fn is_test_only(attrs: &[Attribute]) -> bool {
    attrs.iter().any(|attr| {
        let path = attr.path();
        path.segments
            .last()
            .is_some_and(|last| last.ident == "test")
            || (path.is_ident("cfg") && attr.parse_args::<Meta>().is_ok_and(|m| requires_test(&m)))
    })
}

fn requires_test(meta: &Meta) -> bool {
    match meta {
        Meta::Path(path) => path.is_ident("test"),
        Meta::List(list) if list.path.is_ident("all") => list
            .parse_args_with(Punctuated::<Meta, Token![,]>::parse_terminated)
            .is_ok_and(|items| items.iter().any(requires_test)),
        _ => false,
    }
}

/// The file behind `mod name;` declared in `file`, and the directory its own
/// child modules resolve against (rustc's rules: `#[path]` is relative to the
/// declaring file's directory, or to the module directory inside an inline
/// module; a `#[path]` file owns its directory like `mod.rs`).
fn module_file(
    file: &Path,
    child_dir: &Path,
    inline: bool,
    module: &ItemMod,
) -> Result<(PathBuf, PathBuf), String> {
    let name = module.ident.to_string();
    let explicit = module.attrs.iter().find_map(|attr| match &attr.meta {
        Meta::NameValue(pair) if pair.path.is_ident("path") => match &pair.value {
            Expr::Lit(lit) => match &lit.lit {
                Lit::Str(value) => Some(value.value()),
                _ => None,
            },
            _ => None,
        },
        _ => None,
    });
    if let Some(relative) = explicit {
        let base = if inline {
            child_dir.to_path_buf()
        } else {
            file.parent().map(Path::to_path_buf).unwrap_or_default()
        };
        let path = base.join(relative);
        let dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        return Ok((path, dir));
    }
    let flat = child_dir.join(format!("{name}.rs"));
    if flat.is_file() {
        return Ok((flat, child_dir.join(&name)));
    }
    let nested = child_dir.join(&name).join("mod.rs");
    if nested.is_file() {
        return Ok((nested, child_dir.join(&name)));
    }
    Err(format!(
        "{}: no file for `mod {name};` (looked for {} and {})",
        file.display(),
        flat.display(),
        nested.display()
    ))
}

/// Root re-exports (`pub use tick::Foo as Bar;`) name a top-level module.
fn root_aliases(
    modules: &BTreeMap<String, usize>,
    tree: &UseTree,
    module: Option<usize>,
    aliases: &mut BTreeMap<String, usize>,
) {
    match tree {
        UseTree::Path(path) => {
            let segment = path.ident.to_string();
            let module = match module {
                Some(module) => Some(module),
                None if segment == "crate" || segment == "self" => None,
                None => match modules.get(&segment) {
                    Some(&index) => Some(index),
                    None => return,
                },
            };
            root_aliases(modules, &path.tree, module, aliases);
        }
        UseTree::Name(name) => {
            if let Some(module) = module {
                aliases.insert(name.ident.to_string(), module);
            }
        }
        UseTree::Rename(rename) => {
            if let Some(module) = module {
                aliases.insert(rename.rename.to_string(), module);
            }
        }
        UseTree::Glob(_) => {}
        UseTree::Group(group) => {
            for tree in &group.items {
                root_aliases(modules, tree, module, aliases);
            }
        }
    }
}

struct Resolver {
    modules: BTreeMap<String, usize>,
    aliases: BTreeMap<String, usize>,
}

impl Resolver {
    fn root_name(&self, name: &str) -> Option<usize> {
        self.modules
            .get(name)
            .or_else(|| self.aliases.get(name))
            .copied()
    }
}

struct Collector<'a> {
    resolver: &'a Resolver,
    facts: &'a mut [ModuleFacts],
    /// Index into `facts` the code being visited belongs to.
    current: usize,
    /// Module path below the crate root, e.g. `["tick", "players"]`.
    scope: Vec<String>,
    inline_depth: usize,
    file: PathBuf,
    child_dir: PathBuf,
    /// Inclusive line ranges of skipped test items in `file`.
    test_lines: Vec<(usize, usize)>,
    errors: &'a mut Vec<String>,
}

impl Collector<'_> {
    fn walk_file(&mut self, file: &syn::File, lines: usize, path: &Path, child_dir: PathBuf) {
        if is_test_only(&file.attrs) {
            return;
        }
        let file_path = std::mem::replace(&mut self.file, path.to_path_buf());
        let dir = std::mem::replace(&mut self.child_dir, child_dir);
        let tests = std::mem::take(&mut self.test_lines);
        let depth = std::mem::take(&mut self.inline_depth);
        for item in &file.items {
            self.visit_item(item);
        }
        let skipped = covered_lines(&mut self.test_lines);
        self.facts[self.current].lines += lines.saturating_sub(skipped);
        self.file = file_path;
        self.child_dir = dir;
        self.test_lines = tests;
        self.inline_depth = depth;
    }

    fn skip(&mut self, span: Span) {
        self.test_lines.push((span.start().line, span.end().line));
    }

    fn count(&mut self, counter: Counter) {
        self.facts[self.current].counts[counter as usize] += 1;
    }

    fn literal(&mut self, lit: &LitStr) {
        if lit.value().starts_with("minecraft:") {
            self.count(Counter::VanillaIdLiteral);
        }
    }

    /// Records a reference to another top-level module. `segments` is a path
    /// as written; only `crate::` and `super::` paths can leave the current
    /// top-level module (`self::` stays inside it, and a bare name is a local
    /// or extern item, or an import whose `use` was already recorded).
    fn reference<'s>(&mut self, mut segments: impl Iterator<Item = &'s str>) {
        if self.scope.is_empty() {
            return; // `lib.rs` is not a graph node.
        }
        let mut depth = self.scope.len();
        let mut next = segments.next();
        match next {
            Some("crate") => {
                depth = 0;
                next = segments.next();
            }
            Some("super") => {
                while next == Some("super") {
                    let Some(up) = depth.checked_sub(1) else {
                        return;
                    };
                    depth = up;
                    next = segments.next();
                }
            }
            _ => return,
        }
        if depth > 0 {
            return; // Still inside the current top-level module.
        }
        let Some(target) = next.and_then(|name| self.resolver.root_name(name)) else {
            return;
        };
        if target != self.current {
            *self.facts[self.current].refs.entry(target).or_default() += 1;
        }
    }

    fn use_tree(&mut self, tree: &UseTree, prefix: &mut Vec<String>) {
        match tree {
            UseTree::Path(path) => {
                prefix.push(path.ident.to_string());
                self.use_tree(&path.tree, prefix);
                prefix.pop();
            }
            UseTree::Name(name) => {
                prefix.push(name.ident.to_string());
                self.reference(prefix.iter().map(String::as_str));
                prefix.pop();
            }
            UseTree::Rename(rename) => {
                prefix.push(rename.ident.to_string());
                self.reference(prefix.iter().map(String::as_str));
                prefix.pop();
            }
            UseTree::Glob(_) => self.reference(prefix.iter().map(String::as_str)),
            UseTree::Group(group) => {
                for tree in &group.items {
                    self.use_tree(tree, prefix);
                }
            }
        }
    }

    /// Fallback for macro bodies that are not comma-separated expressions.
    /// Token shapes stand in for the expression checks here: `x.path() ==`
    /// or `!=` (left operand only: a right operand has no token boundary),
    /// `merge_block_writes.push(..)` and `panic!`.
    fn macro_tokens(&mut self, tokens: TokenStream) {
        let tokens: Vec<TokenTree> = tokens.into_iter().collect();
        let at = |offset: usize| tokens.get(offset);
        let mut index = 0;
        while index < tokens.len() {
            match &tokens[index] {
                TokenTree::Group(group) => self.macro_tokens(group.stream()),
                TokenTree::Literal(literal) => {
                    if let Lit::Str(lit) = Lit::new(literal.clone()) {
                        self.literal(&lit);
                    }
                }
                TokenTree::Punct(punct) => {
                    if punct.as_char() == '.'
                        && is_ident(at(index + 1), "path")
                        && parens(at(index + 2)).is_some_and(|args| args.stream().is_empty())
                        && (is_punct(at(index + 3), '=') || is_punct(at(index + 3), '!'))
                        && is_punct(at(index + 4), '=')
                    {
                        self.count(Counter::PathEquality);
                    }
                }
                TokenTree::Ident(ident) => {
                    if ident == "panic" && is_punct(at(index + 1), '!') {
                        self.count(Counter::Panic);
                    }
                    if ident == "merge_block_writes"
                        && is_punct(at(index + 1), '.')
                        && is_ident(at(index + 2), "push")
                        && parens(at(index + 3)).is_some()
                    {
                        self.count(Counter::MergeBlockWritePush);
                    }
                    if ident == "crate" || ident == "super" {
                        let mut segments = vec![ident.to_string()];
                        while is_punct(at(index + 1), ':') && is_punct(at(index + 2), ':') {
                            let Some(TokenTree::Ident(next)) = at(index + 3) else {
                                break;
                            };
                            segments.push(next.to_string());
                            index += 3;
                        }
                        self.reference(segments.iter().map(String::as_str));
                    }
                }
            }
            index += 1;
        }
    }
}

fn is_punct(token: Option<&TokenTree>, ch: char) -> bool {
    matches!(token, Some(TokenTree::Punct(punct)) if punct.as_char() == ch)
}

fn is_ident(token: Option<&TokenTree>, name: &str) -> bool {
    matches!(token, Some(TokenTree::Ident(ident)) if ident == name)
}

fn parens(token: Option<&TokenTree>) -> Option<&Group> {
    match token {
        Some(TokenTree::Group(group)) if group.delimiter() == Delimiter::Parenthesis => Some(group),
        _ => None,
    }
}

/// Number of distinct lines covered by the inclusive `ranges`.
fn covered_lines(ranges: &mut [(usize, usize)]) -> usize {
    ranges.sort_unstable();
    let mut covered = 0;
    let mut next_free = 0;
    for &(start, end) in ranges.iter() {
        let start = start.max(next_free);
        if end >= start {
            covered += end - start + 1;
            next_free = end + 1;
        }
    }
    covered
}

fn item_attrs(item: &Item) -> &[Attribute] {
    match item {
        Item::Const(item) => &item.attrs,
        Item::Enum(item) => &item.attrs,
        Item::ExternCrate(item) => &item.attrs,
        Item::Fn(item) => &item.attrs,
        Item::ForeignMod(item) => &item.attrs,
        Item::Impl(item) => &item.attrs,
        Item::Macro(item) => &item.attrs,
        Item::Mod(item) => &item.attrs,
        Item::Static(item) => &item.attrs,
        Item::Struct(item) => &item.attrs,
        Item::Trait(item) => &item.attrs,
        Item::TraitAlias(item) => &item.attrs,
        Item::Type(item) => &item.attrs,
        Item::Union(item) => &item.attrs,
        Item::Use(item) => &item.attrs,
        _ => &[],
    }
}

fn impl_item_attrs(item: &ImplItem) -> &[Attribute] {
    match item {
        ImplItem::Const(item) => &item.attrs,
        ImplItem::Fn(item) => &item.attrs,
        ImplItem::Type(item) => &item.attrs,
        ImplItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

fn trait_item_attrs(item: &TraitItem) -> &[Attribute] {
    match item {
        TraitItem::Const(item) => &item.attrs,
        TraitItem::Fn(item) => &item.attrs,
        TraitItem::Type(item) => &item.attrs,
        TraitItem::Macro(item) => &item.attrs,
        _ => &[],
    }
}

fn is_path_call(expr: &Expr) -> bool {
    match expr {
        Expr::MethodCall(call) => call.method == "path" && call.args.is_empty(),
        Expr::Paren(inner) => is_path_call(&inner.expr),
        _ => false,
    }
}

fn names_merge_block_writes(expr: &Expr) -> bool {
    match expr {
        Expr::Field(field) => {
            matches!(&field.member, Member::Named(name) if name == "merge_block_writes")
        }
        Expr::Path(path) => path.path.is_ident("merge_block_writes"),
        Expr::Paren(inner) => names_merge_block_writes(&inner.expr),
        _ => false,
    }
}

fn is_panic_path(path: &syn::Path) -> bool {
    let mut names = path
        .segments
        .iter()
        .map(|segment| segment.ident.to_string());
    match (names.next(), names.next(), names.next()) {
        (Some(only), None, None) => only == "panic",
        (Some(krate), Some(name), None) => (krate == "std" || krate == "core") && name == "panic",
        _ => false,
    }
}

impl<'ast> Visit<'ast> for Collector<'_> {
    fn visit_attribute(&mut self, _: &'ast Attribute) {}

    fn visit_visibility(&mut self, _: &'ast Visibility) {}

    fn visit_item(&mut self, item: &'ast Item) {
        if is_test_only(item_attrs(item)) {
            self.skip(item.span());
            return;
        }
        visit::visit_item(self, item);
    }

    fn visit_impl_item(&mut self, item: &'ast ImplItem) {
        if is_test_only(impl_item_attrs(item)) {
            self.skip(item.span());
            return;
        }
        visit::visit_impl_item(self, item);
    }

    fn visit_trait_item(&mut self, item: &'ast TraitItem) {
        if is_test_only(trait_item_attrs(item)) {
            self.skip(item.span());
            return;
        }
        visit::visit_trait_item(self, item);
    }

    fn visit_field(&mut self, field: &'ast Field) {
        if is_test_only(&field.attrs) {
            self.skip(field.span());
            return;
        }
        visit::visit_field(self, field);
    }

    fn visit_variant(&mut self, variant: &'ast Variant) {
        if is_test_only(&variant.attrs) {
            self.skip(variant.span());
            return;
        }
        visit::visit_variant(self, variant);
    }

    fn visit_arm(&mut self, arm: &'ast Arm) {
        if is_test_only(&arm.attrs) {
            self.skip(arm.span());
            return;
        }
        visit::visit_arm(self, arm);
    }

    fn visit_local(&mut self, local: &'ast Local) {
        if is_test_only(&local.attrs) {
            self.skip(local.span());
            return;
        }
        visit::visit_local(self, local);
    }

    fn visit_stmt(&mut self, stmt: &'ast Stmt) {
        if let Stmt::Macro(mac) = stmt
            && is_test_only(&mac.attrs)
        {
            self.skip(mac.span());
            return;
        }
        visit::visit_stmt(self, stmt);
    }

    fn visit_item_mod(&mut self, module: &'ast ItemMod) {
        let name = module.ident.to_string();
        let parent = self.current;
        if self.scope.is_empty() {
            match self.resolver.modules.get(&name) {
                Some(&index) => self.current = index,
                None => {
                    self.skip(module.span()); // A `*_tests.rs` file.
                    return;
                }
            }
        }
        match &module.content {
            Some((_, items)) => {
                self.scope.push(name.clone());
                self.inline_depth += 1;
                let dir = self.child_dir.join(&name);
                let dir = std::mem::replace(&mut self.child_dir, dir);
                for item in items {
                    self.visit_item(item);
                }
                self.child_dir = dir;
                self.inline_depth -= 1;
                self.scope.pop();
            }
            None => match module_file(&self.file, &self.child_dir, self.inline_depth > 0, module) {
                Ok((path, _)) if is_test_file(&path) => self.skip(module.span()),
                Ok((path, child_dir)) => match parse(&path) {
                    Ok((file, lines)) => {
                        self.scope.push(name);
                        self.walk_file(&file, lines, &path, child_dir);
                        self.scope.pop();
                    }
                    Err(err) => self.errors.push(err),
                },
                Err(err) => self.errors.push(err),
            },
        }
        self.current = parent;
    }

    fn visit_item_use(&mut self, item: &'ast ItemUse) {
        if item.leading_colon.is_none() {
            self.use_tree(&item.tree, &mut Vec::new());
        }
    }

    fn visit_path(&mut self, path: &'ast syn::Path) {
        let leaves_module = path
            .segments
            .first()
            .is_some_and(|first| first.ident == "crate" || first.ident == "super");
        if path.leading_colon.is_none() && leaves_module {
            let names: Vec<String> = path
                .segments
                .iter()
                .map(|segment| segment.ident.to_string())
                .collect();
            self.reference(names.iter().map(String::as_str));
        }
        visit::visit_path(self, path);
    }

    fn visit_lit_str(&mut self, lit: &'ast LitStr) {
        self.literal(lit);
    }

    fn visit_expr_binary(&mut self, expr: &'ast ExprBinary) {
        if matches!(expr.op, BinOp::Eq(_) | BinOp::Ne(_))
            && (is_path_call(&expr.left) || is_path_call(&expr.right))
        {
            self.count(Counter::PathEquality);
        }
        visit::visit_expr_binary(self, expr);
    }

    fn visit_expr_method_call(&mut self, call: &'ast ExprMethodCall) {
        if call.method == "push" && names_merge_block_writes(&call.receiver) {
            self.count(Counter::MergeBlockWritePush);
        }
        visit::visit_expr_method_call(self, call);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if is_panic_path(&mac.path) {
            self.count(Counter::Panic);
        }
        self.visit_path(&mac.path);
        match mac.parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated) {
            Ok(exprs) => {
                for expr in &exprs {
                    Visit::visit_expr(self, expr);
                }
            }
            Err(_) => self.macro_tokens(mac.tokens.clone()),
        }
    }
}
