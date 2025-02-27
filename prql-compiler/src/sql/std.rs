use std::collections::HashMap;
use std::iter::zip;

use anyhow::Result;
use itertools::Itertools;
use once_cell::sync::Lazy;
use sqlparser::ast::{self as sql_ast};

use super::gen_expr::translate_sstring;
use super::Context;
use crate::ast::{pl, rq};
use crate::semantic;

static STD: Lazy<semantic::Module> = Lazy::new(load_std_impl);

fn load_std_impl() -> semantic::Module {
    use crate::parser::parse;
    let std_lib = include_str!("./std_impl.prql");
    let statements = parse(std_lib).unwrap();

    let context = semantic::Context {
        root_mod: semantic::Module::new_root(),
        ..semantic::Context::default()
    };

    let (_, context) = semantic::resolve_only(statements, Some(context)).unwrap();
    let std = context.root_mod.get(&pl::Ident::from_name("std")).unwrap();

    std.kind.clone().into_module().unwrap()
}

pub(super) fn translate_built_in(expr: rq::Expr, ctx: &mut Context) -> Result<sql_ast::Expr> {
    let (name, args) = expr.kind.into_built_in_function().unwrap();
    let name = name.strip_prefix("std.").unwrap();

    let entry = STD.get(&pl::Ident::from_name(name)).unwrap();
    let func_def = entry.kind.as_func_def().unwrap();

    let params = func_def
        .named_params
        .iter()
        .chain(func_def.positional_params.iter())
        .map(|x| x.name.split('.').last().unwrap_or(x.name.as_str()));

    let mut args: HashMap<&str, _> = zip(params, args.into_iter()).collect();

    // body can only be an s-string
    let body = &func_def.body.kind.as_s_string().unwrap();
    let body = body
        .iter()
        .map(|item| {
            match item {
                pl::InterpolateItem::Expr(expr) => {
                    // s-string exprs can only contain idents
                    let ident = expr.kind.as_ident();
                    let ident = ident.as_ref().unwrap();

                    // lookup args
                    let arg = args.remove(ident.name.as_str());
                    pl::InterpolateItem::<rq::Expr>::Expr(Box::new(arg.unwrap()))
                }
                pl::InterpolateItem::String(s) => pl::InterpolateItem::String(s.clone()),
            }
        })
        .collect::<Vec<_>>();

    let s_string = translate_sstring(body, ctx)?;

    Ok(sql_ast::Expr::Identifier(sql_ast::Ident::new(s_string)))
}

#[derive(PartialEq, Eq, Hash, Debug)]
pub(crate) struct FunctionDecl<const ARG_COUNT: usize> {
    pub name: &'static str,
}

impl<const AC: usize> FunctionDecl<AC> {
    const fn new(name: &'static str) -> Self {
        Self { name }
    }
}

// TODO: automatically generate these definitions from std_impl.prql
pub(crate) const STD_MUL: FunctionDecl<2> = FunctionDecl::new("std.mul");
pub(crate) const STD_DIV: FunctionDecl<2> = FunctionDecl::new("std.div");
pub(crate) const STD_MOD: FunctionDecl<2> = FunctionDecl::new("std.mod");
pub(crate) const STD_ADD: FunctionDecl<2> = FunctionDecl::new("std.add");
pub(crate) const STD_SUB: FunctionDecl<2> = FunctionDecl::new("std.sub");
pub(crate) const STD_EQ: FunctionDecl<2> = FunctionDecl::new("std.eq");
pub(crate) const STD_NE: FunctionDecl<2> = FunctionDecl::new("std.ne");
pub(crate) const STD_GT: FunctionDecl<2> = FunctionDecl::new("std.gt");
pub(crate) const STD_LT: FunctionDecl<2> = FunctionDecl::new("std.lt");
pub(crate) const STD_GTE: FunctionDecl<2> = FunctionDecl::new("std.gte");
pub(crate) const STD_LTE: FunctionDecl<2> = FunctionDecl::new("std.lte");
pub(crate) const STD_AND: FunctionDecl<2> = FunctionDecl::new("std.and");
pub(crate) const STD_OR: FunctionDecl<2> = FunctionDecl::new("std.or");
pub(crate) const STD_CONCAT: FunctionDecl<2> = FunctionDecl::new("std.concat");
pub(crate) const STD_NEG: FunctionDecl<1> = FunctionDecl::new("std.neg");
pub(crate) const STD_NOT: FunctionDecl<1> = FunctionDecl::new("std.not");

/// Assumes the expr is:
/// - [rq::ExprKind::BuiltInFunction],
/// - name matches the function decl,
/// - number of arguments matches the function decl.
/// Returns the unpacked arguments. Panics if any of the assumptions are not met.
///
/// This function should probably be called after the expr was validated with [try_unpack_ref].
pub(super) fn unpack<const ARG_COUNT: usize>(
    expr: rq::Expr,
    decl: FunctionDecl<ARG_COUNT>,
) -> [rq::Expr; ARG_COUNT] {
    if let rq::ExprKind::BuiltInFunction { name, args } = expr.kind {
        if decl.name == name {
            return args.try_into().unwrap();
        }
    }
    unreachable!()
}

/// Checks that the expr matches the passed built-in-function.
/// Returns an error if the matched function has wrong number of arguments. This can happen when
/// passing an invalid RQ representation.
pub(super) fn try_unpack<const ARG_COUNT: usize, const X: usize>(
    expr: &rq::Expr,
    decls: [FunctionDecl<ARG_COUNT>; X],
) -> Result<Option<(FunctionDecl<ARG_COUNT>, [&rq::Expr; ARG_COUNT])>> {
    if let rq::ExprKind::BuiltInFunction { name, args } = &expr.kind {
        for decl in decls {
            if decl.name != name {
                continue;
            };

            let args: [&rq::Expr; ARG_COUNT] = args
                .iter()
                .collect_vec()
                .try_into()
                .map_err(|_| anyhow::anyhow!("Bad usage of function {}", decl.name))?;

            return Ok(Some((decl, args)));
        }
    }
    Ok(None)
}
