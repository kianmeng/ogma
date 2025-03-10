use super::*;
use libs::fastrand;

pub fn add_intrinsics(impls: &mut Implementations) {
    add! { impls,
        ("get", TableRow, get_tabrow, Pipeline)
        (get, Pipeline)

        (
            ".",
            None,
            ast::DotOperatorBlock::instrinsic,
            Pipeline,
            ast::DotOperatorBlock::help
        )
        ("\\", in, Pipeline)

        ("len", Str, len_str, Pipeline)
        ("len", Table, len_table, Pipeline)

        (let, Pipeline)

        ("nth", Str, nth_str, Pipeline)
        ("nth", Table, nth_table, Pipeline)

        (rand, Pipeline)
        (range, Pipeline)
        ("Table", table, Pipeline)

        ("to-str", bool, to_str_bool, Pipeline)
        ("to-str", Number, to_str_num, Pipeline)
        ("to-str", Str, to_str_str, Pipeline)
        ("to-str", to_str, Pipeline)

        ("Tuple", tuple, Pipeline)
    };
}

// ------ Get ------------------------------------------------------------------
fn get_tabrow_help() -> HelpMessage {
    HelpMessage {
        desc: "extract a value out of a column in a table row.
optionally specify a default value if the get type does not match"
            .into(),
        params: vec![
            HelpParameter::Required("field".into()),
            HelpParameter::Optional("default".into()),
        ],
        examples: vec![
            HelpExample {
                desc: "get the entry of a table row under the column 'size'",
                code: "ls | filter { get size | > 100 }",
            },
            HelpExample {
                desc: "get all files in the directory, requiring 'type' to return a string",
                code: "ls | filter { get:Str type | = 'file' }",
            },
            HelpExample {
                desc: "sum the size of files, using a default of zero",
                code: "ls | fold 0 { + {\\$row | get size 0} }",
            },
        ],

        ..HelpMessage::new("get")
    }
}

fn get_tabrow_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::TabRow)?;

    let colarg = blk
        .next_arg()?
        .supplied(Type::Nil)?
        .returns(Ty::Str)?
        .concrete()?;
    // this is the default arg: 'get foo 0'
    let get_type = match blk.args_len() {
        1 => blk
            .next_arg()?
            .supplied(Type::Nil)?
            .concrete()
            .map(TableGetType::Default)?,
        // use the type flag
        _ => type_flag(&mut blk)
            .and_then(|ty| {
                // otherwise try to infer the output
                ty.map(Ok).unwrap_or_else(|| {
                    blk.output_ty()
                        .ok_or_else(|| Error::unknown_blk_output_type(blk.blk_tag()))
                })
            })
            .map(TableGetType::Flag)?,
    };

    blk.eval(get_type.ty().clone(), move |x, cx| {
        let trow: TableRow = x.try_into()?;
        table_row_get(&trow, &colarg, &get_type, cx)
    })
}

fn get_help() -> HelpMessage {
    HelpMessage {
        desc: "extract a value out of a data structure".into(),
        params: vec![HelpParameter::Required("field".into())],
        examples: vec![HelpExample {
            desc: "get the x field of a user defined Point type",
            code: "Point 1 3 | get x",
        }],

        ..HelpMessage::new("get")
    }
}

fn get_intrinsic(mut blk: Block) -> Result<Step> {
    let field_arg = blk.next_arg()?.supplied(None)?.concrete()?;
    let (facc, out_ty) = FieldAccessor::construct(blk.in_ty(), &field_arg, blk.op_tag())?;
    blk.eval(out_ty, move |input, cx| {
        facc.get(input).and_then(|x| cx.done(x))
    })
}

enum TableGetType {
    Default(eng::Argument),
    Flag(Type),
}

impl TableGetType {
    fn ty(&self) -> &Type {
        match self {
            TableGetType::Default(x) => x.out_ty(),
            TableGetType::Flag(x) => x,
        }
    }
}

fn table_row_get(
    trow: &TableRow,
    colarg: &eng::Argument,
    ty: &TableGetType,
    cx: Context,
) -> Result<(Value, eng::Environment)> {
    let colname = colarg.resolve(|| Value::Nil, &cx).and_then(Str::try_from)?;
    let idx = trow.idx;
    let entry = trow.entry(colname.as_str(), &colarg.tag)?;
    let v = match ty {
        TableGetType::Default(x) => {
            let entry: Value = entry.into();
            if &entry.ty() != x.out_ty() {
                x.resolve(|| Value::Nil, &cx)
            } else {
                Ok(entry)
            }
        }
        TableGetType::Flag(x) => TableRow::cnv_value(entry, x, idx, &colname, &colarg.tag),
    };

    v.and_then(|x| cx.done(x))
}

struct FieldAccessor(usize);

impl FieldAccessor {
    /// Construct a field accessor for the type `ty`. Returns the accessor and the return type of
    /// the field.
    fn construct(ty: &Type, field_arg: &eng::Argument, err_tag: &Tag) -> Result<(Self, Type)> {
        match ty {
            Ty::Def(tydef) => {
                // TypeDefs can use `get` to access a field, so only works for product types.
                // The field is checked, then the accessor index is passed through for the eval Step
                if !matches!(tydef.structure(), types::TypeVariant::Product(_)) {
                    let mut err = Error::wrong_op_input_type(ty, err_tag);
                    err.help_msg = Some("types with `sum` structure cannot be queried into".into());
                    return Err(err);
                }

                let fields = match tydef.structure() {
                    types::TypeVariant::Product(fields) => fields,
                    _ => unreachable!("just checked that we are on Product type"),
                };

                let field_name = field_arg.extract_literal::<Str>()?.as_str();

                let (idx, field) = fields
                    .iter()
                    .enumerate()
                    .find(|(_, f)| f.name().str() == field_name)
                    .ok_or_else(|| Error::field_not_found(&field_arg.tag, tydef))?;

                let out_ty = field.ty().clone();
                Ok((FieldAccessor(idx), out_ty))
            }
            x => Err(Error::wrong_op_input_type(x, err_tag)),
        }
    }

    fn get(&self, val: Value) -> Result<Value> {
        let mut x: OgmaData = val.try_into()?;
        Ok(if let Some(x) = x.get_mut() {
            x.data.remove(self.0)
        } else {
            x.data()[self.0].clone()
        })
    }
}

// ------ Dot Op ---------------------------------------------------------------
impl ast::DotOperatorBlock {
    fn help() -> HelpMessage {
        HelpMessage {
            desc: "extract a value out of a structure using an infix operator".into(),
            params: vec![HelpParameter::Required("=> $foo.bar".into())],
            examples: vec![
                HelpExample {
                    desc: "extract the x coord of a point structure",
                    code: "$point.x",
                },
                HelpExample {
                    desc: "get the value of a column entry in a TableRow",
                    code: "$table-row.col-name",
                },
                HelpExample {
                    desc: "explicitly constrain output type of a column",
                    code: "$table-row.col-name:Str",
                },
            ],
            ..HelpMessage::new(".")
        }
    }

    /// Consists of 2 terms: `input.field`.
    /// For TableRow input we handle separately
    fn instrinsic(mut blk: Block) -> Result<Step> {
        let input = blk.next_arg()?.supplied(None)?.concrete()?;
        let field = blk.next_arg()?.supplied(Ty::Nil)?;
        match input.out_ty() {
            Ty::TabRow => {
                let colarg = field.returns(Ty::Str)?.concrete()?;
                let ty = blk
                    .output_ty()
                    .ok_or_else(|| Error::unknown_blk_output_type(blk.blk_tag()))?;
                let ty = TableGetType::Flag(ty);

                blk.eval(ty.ty().clone(), move |lhs_input, cx| {
                    let trow: TableRow = input.resolve(|| lhs_input, &cx)?.try_into()?;
                    table_row_get(&trow, &colarg, &ty, cx)
                })
            }
            x => {
                let field = field.concrete()?;
                let (facc, out_ty) = FieldAccessor::construct(x, &field, blk.op_tag())?;

                blk.eval(out_ty, move |lhs_input, cx| {
                    let input = input.resolve(|| lhs_input, &cx)?;
                    facc.get(input).and_then(|x| cx.done(x))
                })
            }
        }
    }
}

// ------ Input ----------------------------------------------------------------
fn in_help() -> HelpMessage {
    HelpMessage {
        desc: "sets the input value for the next pipeline block".into(),
        params: vec![HelpParameter::Required("input".into())],
        examples: vec![
            HelpExample {
                desc: "feed in a number",
                code: "\\ 3.14",
            },
            HelpExample {
                desc: "feed in a string",
                code: "\\ 'hello, world!'",
            },
        ],
        ..HelpMessage::new("\\")
    }
}

fn in_intrinsic(mut blk: Block) -> Result<Step> {
    let arg = blk.next_arg()?.supplied(None)?.concrete()?;
    blk.assert_output(arg.out_ty().clone());
    blk.eval(arg.out_ty().clone(), move |val, cx| {
        arg.resolve(|| val, &cx).and_then(|x| cx.done(x))
    })
}

// ------ Length ---------------------------------------------------------------
fn len_str_help() -> HelpMessage {
    HelpMessage {
        desc: "return the number of characters in a string".into(),
        examples: vec![HelpExample {
            desc: "length of a string",
            code: "\\ 'Hello, 🌎!' | len",
        }],
        ..HelpMessage::new("len")
    }
}

fn len_str_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Str)?;
    blk.assert_output(Ty::Num);

    blk.eval_o(|i, cx| {
        Str::try_from(i)
            .map(|s| s.chars().count())
            .map(Number::from)
            .and_then(|x| cx.done_o(x))
    })
}

fn len_table_help() -> HelpMessage {
    HelpMessage {
        desc: "return the number of rows or columns in a table.
length **does not include header row**"
            .into(),
        flags: vec![("cols", "return the number of columns in a table")],
        examples: vec![
            HelpExample {
                desc: "return the number of files on the filesystem",
                code: "ls | filter type --Str eq file | len",
            },
            HelpExample {
                desc: "columns in the ls table",
                code: "ls | len --cols",
            },
        ],
        ..HelpMessage::new("len")
    }
}

fn len_table_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Tab)?;
    blk.assert_output(Ty::Num);

    let cols = blk.get_flag("cols").is_some();
    blk.eval_o(move |t, cx| {
        Table::try_from(t)
            .map(|t| {
                if cols {
                    t.cols_len()
                } else {
                    t.rows_len().saturating_sub(1)
                }
            })
            .map(Number::from)
            .and_then(|x| cx.done_o(x))
    })
}

// ------ Let ------------------------------------------------------------------
fn let_help() -> HelpMessage {
    HelpMessage {
        desc: "assign variable identifiers to expression results
each binding takes the form `<expr> $var`
optionally a final `$var` can be specified which assigns the input
to `$var` and throughputs the input as the output
variables are scoped to within the expression they are defined"
            .into(),
        params: vec![
            HelpParameter::Optional("<expr-1> $var-1".into()),
            HelpParameter::Optional("<expr-2> $var-2".into()),
            HelpParameter::Required("...".into()),
            HelpParameter::Optional("$var-final".into()),
        ],
        examples: vec![
            HelpExample {
                desc: "assign $x to the number 5",
                code: "\\ 5 | let $x",
            },
            HelpExample {
                desc: "assign $x to 1, $y to 2, $z to 3",
                code: "\\ 6 | let {- 5} $x {/ 3} $y {* 0.5} $z",
            },
            HelpExample {
                desc: "assign $x to double input, assign $y to input and pass through",
                code: "let {* 2} $x $y",
            },
        ],
        ..HelpMessage::new("let")
    }
}

fn let_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_adds_vars(false);
    blk.assert_output(blk.in_ty().clone()); // let always passes through the input

    type Binding = (eng::Variable, eng::Argument);

    // detect if the trailing argument is a command (expr) node
    // this could be because of a forgotten pipe
    let forgotten_pipe = blk
        .peek_last_arg_node()
        .filter(|n| n.is_expr(blk.compiler().ag()))
        .map(|n| {
            let g = blk.compiler().ag();
            let etag = n.tag(g);
            let ptag = n.op(g).blk_tag(g);
            format!(
                "maybe you forgot a pipe: `{}| {}`?",
                &ptag.line[ptag.start..etag.start],
                etag
            )
        });

    fn build_bindings(blk: &mut Block) -> Result<Vec<Binding>> {
        blk.oblige_args_supplied_tys(None); // we cheat a little bit here, arg nodes do not need to be obliged
        let mut bindings = Vec::with_capacity(blk.args_len() / 2);
        while blk.args_len() > 1 {
            let e = blk.next_arg()?.supplied(None)?.concrete()?;
            let argnode = blk.next_arg()?.node();
            let v = blk.create_var_ref(argnode, e.out_ty().clone())?;
            bindings.push((v, e));
        }
        Ok(bindings)
    }

    let bindings = build_bindings(&mut blk).map_err(|mut e| {
        let h = match (e.help_msg.take(), forgotten_pipe) {
            (Some(a), Some(b)) => Some(a + "\n    help: " + &b),
            (Some(a), None) => Some(a),
            (None, b) => b,
        };
        e.help_msg = h;
        e
    })?;

    // if there is a trailing binding, the input is bound to that variable, and passed through the
    // block as the output. if not, `let` returns the input type!

    let ty = blk.in_ty().clone();

    let trailing_binding = if blk.args_len() > 0 {
        let argnode = blk.next_arg()?.node();
        let v = blk.create_var_ref(argnode, ty.clone())?;
        Some(v)
    } else {
        None
    };

    blk.assert_vars_added();

    fn bind_vars(bindings: &[Binding], value: &Value, cx: &mut Context) -> Result<()> {
        for (var, e) in bindings {
            let v = e.resolve(|| value.clone(), cx)?;
            var.set_data(&mut cx.env, v);
        }
        Ok(())
    }

    blk.eval(ty, move |value, mut cx| {
        bind_vars(&bindings, &value, &mut cx)?;
        if let Some(trailing_var) = &trailing_binding {
            trailing_var.set_data(&mut cx.env, value.clone());
        }
        cx.done(value)
    })
}

// ------ Nth ------------------------------------------------------------------
fn nth_str_help() -> HelpMessage {
    HelpMessage {
        desc: "retrieves the nth character of a string".into(),
        params: vec![HelpParameter::Required("index".into())],
        examples: vec![HelpExample {
            desc: "get the 10th character of a string",
            code: "\\ 'Hello, world!' | nth 10",
        }],
        ..HelpMessage::new("nth")
    }
}

fn nth_str_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Str)?;
    blk.assert_output(Ty::Str);

    let n = blk
        .next_arg()?
        .supplied(None)?
        .returns(Ty::Num)?
        .concrete()?;
    blk.eval_o::<_, Str>(move |string, cx| {
        let nth = n
            .resolve(|| string.clone(), &cx)
            .and_then(|v| cnv_num_to_uint::<usize>(v, &n.tag))?;
        Str::try_from(string)
            .and_then(|s| {
                s.chars().nth(nth).ok_or_else(|| {
                    Error::eval(
                        &n.tag,
                        "index is outside string bounds",
                        format!("this resolves to `{}`", nth),
                        None,
                    )
                })
            })
            .map(Str::from)
            .and_then(|x| cx.done_o(x))
    })
}

fn nth_table_help() -> HelpMessage {
    HelpMessage {
        desc: "retrieves the nth row and applies the expression".into(),
        params: vec![
            HelpParameter::Required("index".into()),
            HelpParameter::Required("expr".into()),
        ],
        examples: vec![
            HelpExample {
                desc: "get the first row of a table",
                code: "nth 0 {get col-name}",
            },
            HelpExample {
                desc: "get the 2nd last row of a table",
                code: "nth {len | - 2} {get col-name}",
            },
        ],
        ..HelpMessage::new("nth")
    }
}

fn nth_table_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Tab)?;

    let n = blk
        .next_arg()?
        .supplied(None)?
        .returns(Ty::Num)?
        .concrete()?;
    let expr = blk.next_arg()?.supplied(Ty::TabRow)?.concrete()?;
    let oty = expr.out_ty().clone();

    blk.eval(oty, move |table, cx| {
        // nth is adj by one to account for header
        let nth = n
            .resolve(|| table.clone(), &cx)
            .and_then(|v| cnv_num_to_uint::<usize>(v, &n.tag))?;
        let table = Table::try_from(table)?;
        if nth + 1 >= table.rows_len() {
            return Err(Error::eval(
                &n.tag,
                "index is outside table bounds",
                format!("this resolves to `{}`", nth),
                None,
            ));
        }

        let trow = TableRow::new(table, Default::default(), nth + 1);
        expr.resolve(|| trow.into(), &cx).and_then(|v| cx.done(v))
    })
}

// ------ Rand -----------------------------------------------------------------
fn rand_help() -> HelpMessage {
    HelpMessage {
        desc: "return a random number
rand has four ways of calling:
1. Without arguments: this returns a number (0,1],
2. With one argument: this returns a number (0,to],
3. With two arguments: this returns a number (from,to],
4. With three arguments: this returns a table populated with random numbers (from,to]"
            .into(),
        params: vec![
            HelpParameter::Optional("from".into()),
            HelpParameter::Optional("to".into()),
            HelpParameter::Optional("length".into()),
        ],
        examples: vec![
            HelpExample {
                desc: "random integer from 0 to 9",
                code: "rand 0 10 | floor",
            },
            HelpExample {
                desc: "create 10 random numbers",
                code: "rand 0 1 10",
            },
        ],
        ..HelpMessage::new("rand")
    }
}

fn rand_intrinsic(mut blk: Block) -> Result<Step> {
    let args = blk.args_len();

    // assert the return types
    if args == 3 {
        blk.assert_output(Ty::Tab);
    } else {
        blk.assert_output(Ty::Num);
    };

    let mut next_num = || {
        blk.next_arg()
            .and_then(|x| x.supplied(None))
            .and_then(|x| x.returns(Ty::Num))
            .and_then(|x| x.concrete())
            .map(Some)
    };

    let (from, to, len) = match args {
        1 => (None, next_num()?, None),
        2 => (next_num()?, next_num()?, None),
        3 => (next_num()?, next_num()?, next_num()?),
        _ => (None, None, None),
    };

    fn bnd(arg: Option<&eng::Argument>, i: &mut Value, cx: &Context, def: f64) -> Result<f64> {
        match arg {
            Some(x) => Ok(Number::try_from(x.resolve(|| i.clone(), cx)?)?.as_f64()),
            None => Ok(def),
        }
    }

    let tag = blk.op_tag().clone();

    if args == 3 {
        let len = len.unwrap();
        blk.eval_o(move |mut i, cx| {
            let f = bnd(from.as_ref(), &mut i, &cx, 0.0)?;
            let t = bnd(to.as_ref(), &mut i, &cx, 1.0)?;
            let d = t - f;
            let len: usize = cnv_num_to_uint(len.resolve(|| i, &cx)?, &len.tag)?;
            check_from_lt_to(f, t, &tag)?;
            let mut table = InnerTable::new();
            let rng = fastrand::Rng::new();
            table
                .add_col(once(o("rand")).chain(repeat_with(|| rng.f64() * d + f).take(len).map(n)));
            cx.done_o(Table::from(table))
        })
    } else {
        blk.eval_o(move |mut i, cx| {
            let f = bnd(from.as_ref(), &mut i, &cx, 0.0)?;
            let t = bnd(to.as_ref(), &mut i, &cx, 1.0)?;
            let d = t - f;
            check_from_lt_to(f, t, &tag)?;
            cx.done_o(Number::from(fastrand::f64() * d + f))
        })
    }
}

fn check_from_lt_to(from: f64, to: f64, tag: &Tag) -> Result<()> {
    if from >= to {
        Err(Error::eval(
            tag,
            format!("from must be less than to. found from: {} to: {}", from, to),
            None,
            None,
        ))
    } else {
        Ok(())
    }
}

// ------ Range ----------------------------------------------------------------
fn range_help() -> HelpMessage {
    HelpMessage {
        desc: "create a single column table of integers (from,to]
`from` is inclusive, `to` is exclusive
`to` can be omitted if input is a number"
            .into(),
        params: vec![
            HelpParameter::Required("from".into()),
            HelpParameter::Optional("to".into()),
        ],
        examples: vec![
            HelpExample {
                desc: "integers from 0 to 9",
                code: "range 0 10",
            },
            HelpExample {
                desc: "the five preceding numbers",
                code: "\\ 10 | range - 5",
            },
        ],
        ..HelpMessage::new("range")
    }
}

fn range_intrinsic(mut blk: Block) -> Result<Step> {
    fn table_range(from: u128, to: u128) -> Table {
        let mut t = vec![vec![o("i")]];
        t.par_extend((from..to).into_par_iter().map(|x| vec![n(x)]));
        Table::from(::table::Table::from(t))
    }

    blk.assert_output(Type::Tab);

    let from = blk
        .next_arg()?
        .supplied(None)?
        .returns(Type::Num)?
        .concrete()?;
    let alen = blk.args_len();
    match (alen, blk.in_ty()) {
        (0, Ty::Num) => {
            let blktag = blk.blk_tag().clone();
            blk.eval_o(move |input, cx| {
                let from = from
                    .resolve(|| input.clone(), &cx)
                    .and_then(|n| cnv_num_to_uint(n, &from.tag))?;
                let to = cnv_num_to_uint(input, &blktag)?;
                cx.done_o(table_range(from, to))
            })
        }
        _ => {
            let to = blk
                .next_arg()?
                .supplied(None)?
                .returns(Type::Num)?
                .concrete()?;
            blk.eval_o(move |input, cx| {
                let from = from
                    .resolve(|| input.clone(), &cx)
                    .and_then(|n| cnv_num_to_uint(n, &from.tag))?;
                let to = to
                    .resolve(|| input.clone(), &cx)
                    .and_then(|n| cnv_num_to_uint(n, &to.tag))?;
                cx.done_o(table_range(from, to))
            })
        }
    }
}

// ------ Table ctor -----------------------------------------------------------
fn table_help() -> HelpMessage {
    variadic_help(
        "Table",
        "create an empty table with the given table headers",
        vec![
            HelpExample {
                desc: "create an empty table",
                code: "Table",
            },
            HelpExample {
                desc: "create table with the headers 'Foo' and 'Bar'",
                code: "Table 'Foo' 'Bar'",
            },
        ],
    )
}

fn table_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_output(Ty::Tab);

    // table takes zero or more arguments that resolve to Str (header name)
    let mut names = Vec::with_capacity(blk.args_len());
    for _ in 0..blk.args_len() {
        names.push(
            blk.next_arg()?
                .supplied(None)?
                .returns(Ty::Str)?
                .concrete()?,
        );
    }

    blk.eval_o(move |i, cx| {
        let mut t = table::Table::new();
        for name in &names {
            t.add_col(once(name.resolve(|| i.clone(), &cx)?));
        }
        cx.done_o(Table::from(t))
    })
}

// ------ To Str ---------------------------------------------------------------
fn to_str_bool_help() -> HelpMessage {
    HelpMessage {
        desc: "return a boolean as 'true' or 'false'".into(),
        ..HelpMessage::new("to-str")
    }
}

fn to_str_bool_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Bool)?;
    blk.assert_output(Ty::Str);

    blk.eval_o(|v, c| c.done_o(Str::from(bool::try_from(v)?.to_string())))
}

fn to_str_num_help() -> HelpMessage {
    HelpMessage {
        desc: "format the number as a string".into(),
        params: vec![HelpParameter::Optional("fmt".into())],
        examples: vec![HelpExample {
            desc: "format a number as a percentage",
            code: "\\ 0.4123 | to-str '[.2%]'",
        }],
        ..HelpMessage::new("to-str")
    }
}

fn to_str_num_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Num)?;
    blk.assert_output(Ty::Str);

    let fmt = if blk.args_len() == 0 {
        None
    } else {
        let f = blk
            .next_arg()?
            .supplied(None)?
            .returns(Ty::Str)?
            .concrete()?;
        Some(f.extract_literal::<Str>()?.parse::<numfmt::Formatter>().map_err(|e| {
                    Error {
                        cat: err::Category::Parsing,
                        desc: format!("invalid format string: {}", e),
                        traces: vec![err::Trace::from_tag(&f.tag, Some("invalid format string".into()))],
                        help_msg: Some("Number formatting syntax can be found at
          <https://daedalus.report/d/docs/ogma.book/05%20syntax%20and%20semantics/5.4%20number%20formatting.md?pwd-raw=docs>".into()),
                        hard: true,
                    }
                })?)
    };

    blk.eval_o(move |v, cx| {
        let n = Number::try_from(v)?;
        let s = fmt
            .clone()
            .as_mut()
            .map(|f| f.fmt(n.as_f64()).to_string())
            .unwrap_or_else(|| n.to_string());
        cx.done_o(Str::from(s))
    })
}

fn to_str_str_help() -> HelpMessage {
    HelpMessage {
        desc: "pass through the string value".into(),
        ..HelpMessage::new("to-str")
    }
}

fn to_str_str_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_input(&Ty::Str)?;
    blk.assert_output(Ty::Str);

    blk.eval_o(|v, c| c.done_o(Str::try_from(v)?))
}

fn to_str_help() -> HelpMessage {
    HelpMessage {
        desc: "convert the input into a string".into(),
        ..HelpMessage::new("to-str")
    }
}

fn to_str_intrinsic(mut blk: Block) -> Result<Step> {
    blk.assert_output(Ty::Str);

    blk.eval_o(|v, cx| {
        cx.done_o(print::fmt_cell(
            &Entry::from(v),
            &mut numfmt::Formatter::default(),
        ))
    })
}

// ------ Tuple ----------------------------------------------------------------
fn tuple_help() -> HelpMessage {
    variadic_help(
        "Tuple",
        "construct a tuple of the result of each expression
tuples impl `eq` and `cmp` if all its fields also implement `eq` and `cmp`
tuples have unique types: `U_<t0_Ty>-<t1_Ty>_`
access of the fields is using `get t#` with the field number",
        vec![
            HelpExample {
                desc: "create a two element tuple of numbers. type: U_Num-Num_",
                code: "Tuple 1 2",
            },
            HelpExample {
                desc: "create 3 numbers after input. type: U_Num-Num-Num_",
                code: "\\ 3 | Tuple {+ 1} {+ 2} {+ 3}",
            },
            HelpExample {
                desc: "tuples are heterogeneous. type: U_Num-Str-Bool_",
                code: "Tuple 1 'foo' #t",
            },
            HelpExample {
                desc: "get the first and third element",
                code: "Tuple 1 'foo' 2 | + {get t0} {get t2}",
            },
        ],
    )
}

fn tuple_intrinsic(mut blk: Block) -> Result<Step> {
    let len = blk.args_len();
    if len < 2 {
        return Err(Error::insufficient_args(blk.blk_tag(), len as u8, None));
    }
    let mut v = Vec::with_capacity(len);
    for _ in 0..len {
        v.push(blk.next_arg()?.supplied(None)?.concrete()?);
    }

    let ty = Arc::new(Tuple::ty(v.iter().map(|x| x.out_ty().clone()).collect()));

    let oty = Type::Def(ty.clone());

    blk.insert_anon_type_into_compiler(oty.clone());

    blk.assert_output(oty.clone());

    blk.eval(oty, move |input, cx| {
        let mut data = Vec::with_capacity(v.len());
        for arg in &v {
            data.push(arg.resolve(|| input.clone(), &cx)?);
        }
        cx.done(OgmaData::new(ty.clone(), None, data))
    })
}
