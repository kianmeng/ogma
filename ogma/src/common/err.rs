//! Error infrastructure.

use crate::{
    lang::{
        help::*,
        syntax::ast::*,
        types::{Type, TypeDef},
    },
    prelude::*,
};
use ::libs::colored::*;
use std::{
    error, fmt,
    io::{self, Write},
};

macro_rules! colour {
    ($wtr:expr, $colour:expr, $c:ident, $s:literal) => {{
        colour!($wtr, $colour, $c, "{}", $s)
    }};
    ($wtr:expr, $colour:expr, $c:ident, $s:literal, $($tokens:tt)*) => {{
        if $colour {
            write!($wtr, "{}", format!($s, $($tokens)*).$c())
        } else {
            write!($wtr, "{}", format!($s, $($tokens)*))
        }
    }};
}
macro_rules! colourln {
    ($wtr:expr, $colour:expr, $c:ident, $s:literal) => {{
        colourln!($wtr, $colour, $c, "{}", $s)
    }};
    ($wtr:expr, $colour:expr, $c:ident, $s:literal, $($tokens:tt)*) => {{
        if $colour {
            writeln!($wtr, "{}", format!($s, $($tokens)*).$c())
        } else {
            writeln!($wtr, "{}", format!($s, $($tokens)*))
        }
    }};
}

/// Ubiquitous error.
///
/// Errors are printed like so:
/// ```shell
/// Category: description
/// --> location:column-num
///  | source line { }
///  |        ^^^^ short description
/// --> help: help message
/// ```
#[derive(Debug, PartialEq, Default)]
pub struct Error {
    /// Category of error.
    pub cat: Category,
    /// Error description.
    pub desc: String,
    /// Error backtrace.
    pub traces: Vec<Trace>,
    /// Optional help message.
    pub help_msg: Option<String>,
    /// Error should propogate immediately.
    ///
    /// This is a flag to the compiler that the error should be propogated through, exiting the
    /// compiling loop early without further compilation processes.
    /// This is usually done when a error is encountered that is unrelated to typing issues, and
    /// will be an error even if _all_ type information was known.
    pub hard: bool,
}

/// A single trace item for error messages.
#[derive(Debug, PartialEq)]
pub struct Trace {
    /// The defined location.
    pub loc: Location,
    /// The source code.
    pub source: String,
    /// Description of trace.
    pub desc: Option<String>,
    /// Starting position in `source`.
    pub start: usize,
    /// Length of trace element.
    pub len: usize,
}

/// Represent a help message using the [`Error`] infrastructure.
pub fn help_as_error(msg: &HelpMessage, in_ty: Option<&Type>) -> Error {
    use fmt::Write;

    let cmd = msg.cmd.as_str();
    let mut source = "---- Input Type: ".to_string();

    match in_ty {
        Some(t) => write!(&mut source, "{t}"),
        None => write!(&mut source, "<any>"),
    }
    .ok();

    source = source + " ----\n" + &msg.desc + "\n\nUsage:\n => " + cmd;

    for param in &msg.params {
        let brk = matches!(param, HelpParameter::Break);
        if brk {
            write!(source, "\n => {}", cmd).ok();
        }
        if !msg.no_space {
            source.push(' ');
        }
        if !brk {
            param.write(&mut source);
        }
    }

    if !msg.flags.is_empty() {
        source.push_str("\n\nFlags:");
        for (name, desc) in &msg.flags {
            source.push_str("\n --");
            source.push_str(name);
            source.push_str(": ");
            source.push_str(desc);
        }
    }

    if !msg.examples.is_empty() {
        source.push_str("\n\nExamples:");
        for example in &msg.examples {
            write!(source, "\n {}\n => {}\n", example.desc, example.code).ok();
        }
    }

    Error {
        cat: Category::Help,
        desc: format!("`{}`", cmd),
        traces: vec![Trace {
            source,
            ..Default::default()
        }],
        ..Error::default()
    }
}

// ###### ERROR ################################################################
/// Create a single trace item using the tag.
pub fn trace<D: Into<Option<String>>>(tag: &Tag, desc: D) -> Vec<Trace> {
    vec![Trace::from_tag(tag, desc)]
}

impl Error {
    pub(crate) fn add_trace<M>(mut self, tag: &Tag, msg: M) -> Self
    where
        M: Into<Option<String>>,
    {
        let msg = msg.into().unwrap_or_else(|| "invoked here".to_string());
        self.traces.push(Trace::from_tag(tag, msg));
        self
    }

    pub(crate) fn predefined_impl(def: &DefinitionImpl, in_ty: Option<&Type>) -> Self {
        let desc = if let Some(t) = in_ty {
            format!(
                "implementation `{}` for input type {} is predefined by ogma",
                def.name, t
            )
        } else {
            format!(
                "implementation `{}` for unspecified input types is predefined by ogma",
                def.name
            )
        };

        Error {
            cat: Category::Definitions,
            desc,
            traces: trace(&def.name, format!("`{}` defined by ogma", def.name)),
            help_msg: Some(format!(
                "use a different name, or try defining `{}` for a specific input type",
                def.name
            )),
            ..Self::default()
        }
    }

    pub(crate) fn op_not_found(
        op: &Tag,
        inty: Option<&Type>,
        recursion_detected: bool,
        impls: &Implementations,
    ) -> Self {
        fn tystr(x: Option<&Type>) -> String {
            x.map(|x| x.to_string()).unwrap_or_else(|| "<any>".into())
        }

        let ty = impls.iter_op(op.str()).collect::<Vec<_>>();

        let hlp = if recursion_detected {
            "recursion is not supported.
          for alternatives, please see <https://daedalus.report/d/docs/ogma.book/11%20(no)%20recursion.md?pwd-raw=docs>".into()
        } else if ty.is_empty() {
            "view a list of definitions using `def --list`".into()
        } else {
            ty.into_iter().fold(
                format!("`{op}` is implemented for the following input types:"),
                |s, t| s + " " + &tystr(t.ty),
            )
        };

        Error {
            cat: Category::UnknownCommand,
            desc: format!("operation `{}` not defined", op),
            traces: trace(
                op,
                format!("`{}` not defined for input `{}`", op, tystr(inty)),
            ),
            help_msg: Some(hlp),
            hard: true,
        }
    }

    pub(crate) fn impl_not_found(op: &Tag, in_ty: &Type) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!(
                "implementation of `{}` not defined for input type `{}`",
                op, in_ty
            ),
            traces: trace(op, format!("`{}` not implmented for `{}` input", op, in_ty)),
            help_msg: Some("view a list of definitions using `def --list`".into()),
            ..Self::default()
        }
    }

    pub(crate) fn insufficient_args(
        block_tag: &Tag,
        args_count: u8,
        signature: Option<&DefinitionImpl>,
    ) -> Self {
        use std::fmt::Write;

        let mut help_msg = String::from("try using the `--help` flag to view requirements");

        if let Some(impl_) = signature {
            let mut params = impl_.params.iter().fold(String::new(), |mut s, param| {
                write!(&mut s, "{}", param.ident).ok();
                if let Some(ty) = &param.ty {
                    write!(&mut s, ":{}", ty).ok();
                }

                s += " ";

                s
            });
            params.pop(); // remove trailing space

            writeln!(&mut help_msg, ".").ok();
            write!(
                &mut help_msg,
                "          `{}` is defined to accept parameters `({})`",
                impl_.name, params
            )
            .ok();
        }

        Error {
            cat: Category::Semantics,
            desc: format!("expecting more than {} arguments", args_count),
            traces: trace(block_tag, "expecting additional argument(s)".to_string()),
            help_msg: Some(help_msg),
            hard: true, // unrecoverable?
        }
    }

    pub(crate) fn unused_flags<'a, T>(flags: T) -> Self
    where
        T: DoubleEndedIterator<Item = &'a Tag>,
    {
        let delim = ", ";

        let (desc, traces) = flags.rev().fold(
            (String::from("not expecting flags: "), Vec::new()),
            |(desc, mut traces), flag| {
                traces.push(Trace::from_tag(flag, "flag not supported".to_string()));
                (desc + "`" + flag.str() + "`" + delim, traces)
            },
        );

        let desc = desc.trim_end_matches(delim).to_string();

        Error {
            cat: Category::Semantics,
            desc,
            traces,
            help_msg: Some("try using the `--help` flag to view requirements".into()),
            hard: true, // unrecoverable
        }
    }

    pub(crate) fn unused_args<'a, T>(args: T) -> Self
    where
        T: ExactSizeIterator<Item = &'a Tag>,
    {
        let len = args.len();
        let tag = args
            .cloned()
            .reduce(|mut a, b| {
                a.make_mut().start = a.start.min(b.start);
                a.make_mut().end = a.end.max(b.end);
                a
            })
            .unwrap_or_default();
        let msg = if len == 1 {
            "this argument is unnecessary"
        } else {
            "these arguments are unnecessary"
        };

        Error {
            cat: Category::Semantics,
            desc: "too many arguments supplied".into(),
            traces: trace(&tag, Some(msg.into())),
            ..Self::default()
        }
    }

    pub(crate) fn unexp_arg_variant(tag: &Tag, variant: &str) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!("not expecting argument variant `{}`", variant),
            traces: trace(
                tag,
                format!("argument variant `{}` is not supported here", variant),
            ),
            help_msg: Some(
                "commands may require specific argument types, use `--help` to view requirements"
                    .into(),
            ),
            hard: true, // non-recoverable
        }
    }

    pub(crate) fn empty_table<'a, C: Into<Option<&'a str>>>(colname: C, tag: &Tag) -> Self {
        Error {
            cat: Category::Evaluation,
            desc: "empty table".into(),
            traces: match colname.into() {
                Some(x) => trace(tag, format!("`{}` resolves to `{}`", tag.str(), x)),
                None => trace(tag, None),
            },
            ..Self::default()
        }
    }

    pub(crate) fn header_not_found(colname: &str, tag: &Tag) -> Self {
        Error {
            cat: Category::Evaluation,
            desc: format!("header `{}` not found in table", colname),
            traces: trace(tag, format!("`{}` resolves to `{}`", tag.str(), colname)),
            ..Self::default()
        }
    }

    pub(crate) fn row_out_of_bounds(index: usize, tag: &Tag) -> Self {
        Error {
            cat: Category::Evaluation,
            desc: format!("row index `{}` is outside table bounds", index),
            traces: trace(tag, format!("`{}` resolves to {}", tag.str(), index)),
            help_msg: Some("use `len` command to check the size of the table".into()),
            ..Self::default()
        }
    }

    pub(crate) fn unexp_entry_ty<C: fmt::Display>(
        exp: &Type,
        found: &Type,
        row: usize,
        colname: C,
        tag: &Tag,
    ) -> Self {
        Self::eval(
            tag,
            format!(
                "table entry for [row:{},col:'{}'] did not have expected type
expected `{}`, found `{}`",
                row, colname, exp, found
            ),
            None,
            "column entries must have a matching type".to_string(),
        )
    }

    /// For use with pound literals `#t` etc.
    pub(crate) fn unknown_spec_literal(found: char, tag: &Tag) -> Self {
        Self {
            cat: Category::Semantics,
            desc: format!("special literal `{}` not supported", found),
            traces: trace(tag, format!("`{}` not supported", found)),
            hard: true,
            ..Self::default()
        }
    }

    pub(crate) fn eval<D, S, H>(tag: &Tag, desc: D, short_desc: S, help: H) -> Self
    where
        D: Into<String>,
        S: Into<Option<String>>,
        H: Into<Option<String>>,
    {
        Error {
            cat: Category::Evaluation,
            desc: desc.into(),
            traces: trace(tag, short_desc),
            help_msg: help.into(),
            ..Self::default()
        }
    }

    pub(crate) fn io(block: &Tag, err: std::io::Error) -> Self {
        Self::eval(
            block,
            format!("an io error occurred: {}", err),
            String::from("within this block"),
            None,
        )
    }

    /// This is an internal error!
    pub(crate) fn conversion_failed(exp: &Type, found: &Type) -> Self {
        Error {
            cat: Category::Evaluation,
            desc: format!(
                "converting value into `{}` failed, value has type `{}`",
                exp, found
            ),
            help_msg: Some("this is an internal bug, please report it at <https://github.com/kdr-aus/ogma/issues>".into()),
                ..Self::default()
        }
    }

    /// Convert [`Argument`] to `(Tag, &str)` for use with `Error::unexp_arg_variant`.
    pub fn span_arg<'a>(arg: &'a Argument) -> (&'a Tag, &'static str) {
        let tag = arg.tag();
        let s = match arg {
            Argument::Ident(_) => "identifier",
            Argument::Pound(_, _) => "special-literal",
            Argument::Num(_, _) => "number",
            Argument::Var(_) => "variable",
            Argument::Expr(_) => "expression",
        };
        (tag, s)
    }

    /// Pretty print the error.
    ///
    /// If `colourize` is `true` then terminal colouring will be applied to errors.
    pub fn print(&self, colourize: bool, wtr: &mut dyn Write) -> io::Result<()> {
        let c = colourize;

        // description
        {
            match self.cat {
                Category::Internal => colour!(wtr, c, bright_red, "Internal Error"),
                Category::Parsing => colour!(wtr, c, bright_red, "Parsing Error"),
                Category::UnknownCommand => colour!(wtr, c, bright_red, "Unknown Command"),
                Category::Semantics => colour!(wtr, c, bright_red, "Semantics Error"),
                Category::Type => colour!(wtr, c, bright_red, "Typing Error"),
                Category::Evaluation => colour!(wtr, c, bright_red, "Evaluation Error"),
                Category::Definitions => colour!(wtr, c, bright_red, "Definition Error"),
                Category::Help => colour!(wtr, c, bright_yellow, "Help"),
            }?;
            colourln!(wtr, c, bright_white, ": {}", self.desc)?;
        }

        // traces
        for trace in &self.traces {
            trace.print(c, wtr)?;
        }

        // help message
        if let Some(help) = &self.help_msg {
            colour!(wtr, c, bright_purple, "--> help: ")?;
            colourln!(wtr, c, yellow, "{}", help)?;
        }

        Ok(())
    }
}

/// Syntax
impl Error {}

/// Internal
impl Error {
    pub(crate) fn internal_err_help() -> Option<String> {
        let bt = backtrace::Backtrace::new();
        Some(format!(
            "this is an internal bug, please report it at <https://github.com/kdr-aus/ogma/issues>
Please supply this BACKTRACE:
{:?}",
            bt
        ))
    }

    pub(crate) fn incomplete_expr_compilation(expr: &Tag) -> Self {
        Error {
            cat: Category::Internal,
            desc: "expression is yet to be compiled".into(),
            traces: trace(
                expr,
                Some("this expression has not finished compiling".into()),
            ),
            help_msg: Self::internal_err_help(),
            ..Self::default()
        }
    }

    pub(crate) fn ag_init_endless_loop(loop_counter: u32, block_tag: &Tag) -> Self {
        Error {
            cat: Category::Internal,
            desc: format!("AST graph reach {} loops", loop_counter),
            traces: trace(block_tag, None),
            help_msg: Self::internal_err_help(),
            ..Self::default()
        }
    }

    pub(crate) fn unexp_code_injection_output_ty(
        ty: &Type,
        exp_ty: &Type,
        block_tag: &Tag,
    ) -> Self {
        Error {
            cat: Category::Internal,
            desc: "Internal code injection output type does not match expected output type".into(),
            traces: trace(
                block_tag,
                Some(format!(
                    "this block returns '{}', expecting '{}'",
                    ty, exp_ty
                )),
            ),
            help_msg: Self::internal_err_help(),
            ..Self::default()
        }
    }

    pub(crate) fn update_locals_graph(tag: &Tag) -> Self {
        Error {
            cat: Category::Internal,
            desc: "the locals graph has been changed and needs updating".into(),
            traces: trace(tag, None),
            help_msg: Self::internal_err_help(),
            ..Self::default()
        }
    }

    /// Wrap an error coming from a `CodeInjector`.
    pub(crate) fn wrap_code_injection(mut self, blk_tag: &Tag) -> Self {
        self.traces.push(Trace::from_tag(
            blk_tag,
            Some("this block internally injects code".into()),
        ));
        self.cat = Category::Internal;
        self.help_msg = Self::internal_err_help();
        self
    }

    /// Use this to bubble an inference depth reached error.
    pub(crate) fn inference_depth() -> Self {
        Self {
            cat: Category::Type,
            desc: "inference depth reached".to_string(),
            hard: true,
            help_msg: Some(
                "try annotating the input and/or output types you are expecting".to_string(),
            ),
            ..Default::default()
        }
    }

    /// Is this error because of reaching inference depth?
    pub fn is_inference_depth_error(&self) -> bool {
        self.desc.starts_with("inference depth reached")
    }
}

/// Type Errors
impl Error {
    pub(crate) fn type_not_found(ty: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!("type `{}` not defined", ty),
            traces: trace(ty, format!("`{}` not defined", ty)),
            help_msg: Some("view a list of types using `def-ty --list`".into()),
            hard: true, // unrecoverable
        }
    }

    pub(crate) fn unknown_blk_output_type(blk: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: "unable to infer block's output type".into(),
            traces: trace(blk, None),
            ..Self::default()
        }
    }

    pub(crate) fn wrong_op_input_type(ty: &Type, op: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!("`{}` does not support `{}` input data", op, ty),
            traces: trace(op, None),
            help_msg: Some(format!(
                "use `{0} --help` to view requirements. consider implementing `def {0}`",
                op
            )),
            hard: true,
        }
    }

    pub(crate) fn unknown_arg_input_type(arg: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: "unable to infer argument's input type".into(),
            traces: trace(arg, None),
            ..Self::default()
        }
    }

    pub(crate) fn unknown_arg_output_type(arg: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: "unable to infer argument's output type".into(),
            traces: trace(arg, None),
            ..Self::default()
        }
    }

    pub(crate) fn unexp_arg_input_ty(exp: &Type, found: &Type, arg: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!(
                "expecting argument to take input type `{}`, accepts `{}`",
                exp, found
            ),
            traces: trace(arg, format!("this argument accepts type `{}`", found)),
            ..Self::default()
        }
    }

    pub(crate) fn unexp_arg_output_ty(exp: &Type, found: &Type, arg: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!(
                "expecting argument with output type `{}`, found `{}`",
                exp, found
            ),
            traces: trace(arg, format!("this argument returns type `{}`", found)),
            help_msg: Some(
                "commands may require specific argument types, use `--help` to view requirements"
                    .into(),
            ),
            ..Self::default()
        }
    }

    pub(crate) fn field_not_found(field: &Tag, ty: &TypeDef) -> Self {
        fn hlp(ty: &TypeDef) -> Option<String> {
            use types::TypeVariant::*;

            match ty.structure() {
                Sum(_) => None,
                Product(fields) => {
                    let delim = ", ";
                    let list = fields
                        .iter()
                        .fold(String::new(), |s, f| s + f.name().str() + delim);
                    Some(format!(
                        "`{}` has the following fields: {}",
                        ty.name(),
                        list.trim_end_matches(delim)
                    ))
                }
            }
        }

        Error {
            cat: Category::Semantics,
            desc: format!("`{}` does not contain field `{}`", ty.name(), field),
            traces: trace(field, format!("`{}` not found", field)),
            help_msg: hlp(ty),
            hard: true,
        }
    }
}

/// Variable Errors
impl Error {
    pub(crate) fn var_not_found(var: &Tag) -> Self {
        Error {
            cat: Category::Semantics,
            desc: format!("variable `{var}` does not exist"),
            traces: trace(var, format!("`{var}` not in scope")),
            help_msg: Some(
                "variables must be in scope
          variables can be defined using the `let` command"
                    .into(),
            ),
            hard: true, // unrecoverable, variable not found in locals
        }
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut s = Vec::new();
        self.print(false, &mut s)
            .expect("writing to vector should not fail");
        let s = String::from_utf8(s).expect("print should only print valid characters");
        write!(f, "{}", s)
    }
}

impl error::Error for Error {}

// ###### TRACE ################################################################
/// Returns a _blank_ trace. Intended for assistance with initialising.
impl Default for Trace {
    fn default() -> Self {
        Self {
            loc: Location::Shell,
            source: String::new(),
            desc: None,
            start: 0,
            len: 0,
        }
    }
}

impl Trace {
    /// Build the error trace from a tag.
    pub fn from_tag<D: Into<Option<String>>>(tag: &Tag, desc: D) -> Self {
        Self {
            loc: tag.anchor.clone(),
            source: tag.line().into(),
            desc: desc.into(),
            start: tag.start,
            len: tag.len(),
        }
    }

    fn print(&self, colourize: bool, wtr: &mut dyn Write) -> io::Result<()> {
        let c = colourize;
        let Self {
            loc,
            source,
            desc,
            start,
            len,
        } = self;
        let start = *start;
        let len = *len;

        let src_lines = if len > 0 {
            trace_code_lines(source, start, start + len)
        } else {
            source.lines().map(|s| (s, 0, 0)).collect()
        };

        // error location
        let pos = src_lines.iter().map(|(_, x, _)| *x).next().unwrap_or(0);
        colourln!(wtr, c, bright_purple, "--> {}:{}", loc, pos)?;

        // source lines
        for (line, _, _) in &src_lines {
            colour!(wtr, c, bright_purple, " | ")?;
            colourln!(wtr, c, white, "{}", line)?;
        }

        // error location identifier
        use std::cmp::*;
        let (min, max) = src_lines
            .iter()
            .fold((10_000, 0), |(x, y), (_, a, b)| (min(x, *a), max(y, *b)));

        if min < max {
            colour!(wtr, c, bright_purple, " | ")?;
            for _ in 0..min {
                write!(wtr, " ")?;
            }
            let carrots = "^".repeat(max - min);
            colour!(wtr, c, bright_red, "{}", carrots)?;
            if let Some(desc) = &desc {
                colour!(wtr, c, bright_red, " {}", desc)?;
            }
            writeln!(wtr)?;
        }

        Ok(())
    }
}

/// Returns the lines intersecting `start` and `end` byte positions.
/// Each line has a range specifying the _character_ range of visible characters.
/// The first and last lines visible ranges respect the `start` and `end`.
fn trace_code_lines(code: &str, start: usize, end: usize) -> Vec<(&str, usize, usize)> {
    let mut lines = Vec::new();

    for line in code.lines() {
        let offset_start = unsafe { line.as_ptr().offset_from(code.as_ptr()) } as usize;
        let offset_end = offset_start + line.len();

        if offset_end < start || offset_start >= end {
            continue;
        }

        let adj_start = offset_start <= start;
        let adj_end = offset_end >= end;

        let tabsize = |c| if c == '\t' { 4 } else { 1 };

        let s: usize = if adj_start {
            code[offset_start..start].chars().map(tabsize).sum()
        } else {
            line.chars()
                .take_while(|c| c.is_whitespace())
                .map(tabsize)
                .sum()
        };
        let e: usize = if adj_end {
            code[offset_start..end].chars().map(tabsize).sum()
        } else if offset_end == start {
            s + 1
        } else {
            line.trim_end().chars().map(tabsize).sum()
        };

        lines.push((line, s, e));
    }

    lines
}

// ###### STRUCTS ##############################################################
/// Error catgories.
#[derive(Debug, PartialEq)]
pub enum Category {
    /// Internal error. These should not occur.
    Internal,
    /// Parsing error.
    Parsing,
    /// Command is not recognised.
    UnknownCommand,
    /// Semantic error at comp-time.
    Semantics,
    /// Type inference failure.
    Type,
    /// A run-time evaluation error.
    Evaluation,
    /// A definition error.
    Definitions,
    /// A help message (built atop the error infrastructure).
    Help,
}

impl Default for Category {
    fn default() -> Self {
        Category::Internal
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn print(err_trace: &Trace) -> String {
        let mut s = Vec::new();
        err_trace.print(false, &mut s).unwrap();
        String::from_utf8(s).unwrap()
    }

    #[test]
    fn trace_code_lines_test() {
        let f = trace_code_lines;
        assert_eq!(f("Hello", 0, 5), vec![("Hello", 0, 5)]);
        assert_eq!(f("Hello", 1, 3), vec![("Hello", 1, 3)]);
        assert_eq!(f("Hello\nWorld", 6, 11), vec![("World", 0, 5)]);
        assert_eq!(f("Hello\nWorld", 7, 9), vec![("World", 1, 3)]);
        assert_eq!(
            f("Hello\nWorld\nLook here", 2, 16),
            vec![("Hello", 2, 5), ("World", 0, 5), ("Look here", 0, 4),]
        );
    }

    #[test]
    fn trace_code_single_mark() {
        let f = trace_code_lines;
        assert_eq!(f("in | ", 5, 6), vec![("in | ", 5, 6)]);
    }

    #[test]
    fn printing_error_traces_basic() {
        let et = &Trace {
            source: "Hello".into(),
            start: 3,
            len: 2,
            ..Default::default()
        };

        let x = print(et);
        println!("{}", x);
        assert_eq!(
            &x,
            "--> shell:3
 | Hello
 |    ^^
"
        );
    }

    #[test]
    fn printing_error_traces_mutliline_single_span() {
        let et = &Trace {
            source: "Hello
World
This is
A multiline"
                .into(),
            start: 12,
            len: 4,
            ..Default::default()
        };

        let x = print(et);
        println!("{}", x);
        assert_eq!(
            &x,
            "--> shell:0
 | This is
 | ^^^^
"
        );

        let et = &Trace {
            source: "Hello
World
    This is
    A multiline"
                .into(),
            start: 7,
            len: 20,
            ..Default::default()
        };

        let x = print(et);
        println!("{}", x);
        assert_eq!(
            &x,
            "--> shell:1
 | World
 |     This is
 |     A multiline
 |  ^^^^^^^^^^
"
        );
    }

    #[test]
    fn printing_error_traces_mutliline_multi_span() {
        let et = &Trace {
            source: "if { foo {
    bar zog |
    43 |
    }
}"
            .into(),
            start: 18,
            len: 10,
            ..Default::default()
        };

        let x = print(et);
        println!("{}", x);
        assert_eq!(
            &x,
            "--> shell:7
 |     bar zog |
 |     43 |
 |     ^^^^^^^^^
"
        );
    }

    #[test]
    fn single_mark_cmd() {
        let et = &Trace {
            source: "in | ".into(),
            start: 5,
            len: 1,
            ..Default::default()
        };

        let x = print(et);
        println!("{}", x);
        assert_eq!(
            &x,
            "--> shell:5
 | in | 
 |      ^
"
        );
    }

    #[test]
    fn chk_inference_depth() {
        let e = Error::inference_depth();
        assert!(e.is_inference_depth_error());
    }
}
