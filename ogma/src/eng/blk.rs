use super::*;
use graphs::*;

impl<'a> Block<'a> {
    /// The operation (command) tag.
    pub fn op_tag(&self) -> &Tag {
        self.ag[self.node.idx()]
            .op()
            .expect("blocks node is an OP")
            .0
    }

    /// The entire block tag.
    pub fn blk_tag(&self) -> &Tag {
        self.ag[self.node.idx()]
            .op()
            .expect("blocks node is an OP")
            .1
    }

    /// The input [`Type`] of the block.
    pub fn in_ty(&self) -> &Type {
        &self.in_ty
    }

    /// Returns the _number of arguments remaining_.
    pub fn args_len(&self) -> usize {
        self.args.len()
    }

    /// Assert that this block will return the given type.
    ///
    /// Asserting an output type gives the type inferer knowledge about this block's output.
    /// It is recommended to assert this as soon as possible in the implementation, so if
    /// compilation fails, the output information is still captured.
    ///
    /// # Panics
    /// - For debug builds, panics if an output type has already been set on this block.
    /// Only a single assertion should be made in an implementation.
    /// - For debug builds, this assertion is checked against the evalulation output type.
    pub fn assert_output(&mut self, ty: Type) {
        #[cfg(debug_assertions)]
        {
            debug_assert!(
                self.output_ty.is_none(),
                "Block.output_ty cannot be set more than once"
            );
            self.output_ty = Some(ty.clone());
        }

        self.chgs
            .push(graphs::tygraph::Chg::KnownOutput(self.node.idx(), ty).into());
    }

    /// Gets the flag that matches a given name.
    ///
    /// If no name is given with `None`, _the first flag first is returned, if there is one._
    ///
    /// > The flag is **removed** from the flag stack.
    pub fn get_flag<'b, N: Into<Option<&'b str>>>(&mut self, flag: N) -> Option<Tag> {
        match flag.into() {
            Some(name) => self
                .flags
                .iter()
                .position(|x| x.str() == name)
                .map(|i| self.flags.remove(i)),
            None => self.flags.pop(),
        }
    }

    /// See if there is a next argument node, without popping off the stack.
    pub fn peek_next_arg_node(&self) -> Option<graphs::ArgNode> {
        self.args.last().copied()
    }

    /// Assert this argument is a variable and construct a reference to it.
    ///
    /// If the block does not contain a reference to an up-to-date locals, and error is returned.
    ///
    /// **Note** that the variable will be defined at the **_next_** block in the chain, not the
    /// current one.
    ///
    /// # Type safety
    /// The variable will be created expecting the type `ty`. `set_data` only validates types in
    /// debug builds, be sure that testing occurs of code path to avoid UB in release.
    pub fn create_var_ref(&mut self, arg: ArgNode, ty: Type) -> Result<Variable> {
        match &self.ag[arg.idx()] {
            astgraph::AstNode::Var(var) => match arg.op(&self.ag).next(&self.ag) {
                Some(next) => self
                    .lg
                    .new_var(next.idx(), Str::new(var.str()), ty, var.clone())
                    .map_err(|chg| {
                        self.chgs.push(chg.into());
                        Error::update_locals_graph(var)
                    }),
                None => Ok(Variable::noop(var.clone(), ty)),
            },
            x => {
                use astgraph::AstNode::*;
                let v = match x {
                    Ident(_) => "identifier",
                    Num { .. } => "number",
                    Expr(_) => "expression",
                    Var(_) => "variable",
                    Pound { .. } => "special-literal",
                    Op { .. } | Intrinsic { .. } | Def { .. } | Flag(_) => unreachable!(),
                };

                Err(Error::unexp_arg_variant(x.tag(), v))
            }
        }
    }

    /// Create a variable reference not off a specific argument, but by manually specifying the
    /// variable name.
    ///
    /// This is useful for expressions that need to supply more than just the input. For instance
    /// the `fold` command will supply the `$row` variable which is a track of the TableRow.
    ///
    /// Consider using the helper `Block::inject_manual_var_next_arg` which uses the next arguments
    /// argnode.
    pub fn inject_manual_var_into_arg_locals<N>(
        &mut self,
        arg: graphs::ArgNode,
        name: N,
        ty: Type,
    ) -> Result<Variable>
    where
        N: Into<Str>,
    {
        // we define it into the arg node
        let tag = arg.tag(&self.ag);
        self.lg
            .new_var(arg.idx(), name, ty, tag.clone())
            .map_err(|chg| {
                self.chgs.push(chg.into());
                Error::update_locals_graph(tag)
            })
    }

    /// Helper for `Block::inject_manual_var_into_arg_locals` which peeks the next argument on the
    /// stack to use as the `arg` input.
    pub fn inject_manual_var_next_arg<N>(&mut self, name: N, ty: Type) -> Result<Variable>
    where
        N: Into<Str>,
    {
        let n = self
            .peek_next_arg_node()
            .ok_or_else(|| Error::insufficient_args(self.op_tag(), self.args_count, None))?;
        self.inject_manual_var_into_arg_locals(n, name, ty)
    }
}

/// Evalulation functions.
impl<'a> Block<'a> {
    /// Most flexible evaluation option, but also most brittle.
    ///
    /// **BE EXTRA CAREFUL WITH THE `out_ty` THAT IT MATCHES THE EVAL VALUE.
    /// It is recommended to thoroughly test code paths through here to ensure type validity.**
    ///
    /// # Usage
    /// - Input value ([`Value`]) should be cast to expected input type (use `.try_into()?`).
    /// - Arguments can use this type if they are expecting the blocks input.
    ///
    /// ## Arguments
    /// Arguments **do not need to use blocks input**. If no input type is needed, the argument can
    /// be built with `Type::Nil` and `Value::Nil` can be passed on through!
    pub fn eval<F>(self, out_ty: Type, f: F) -> Result<Step>
    where
        F: Fn(Value, Context) -> StepR,
        F: Func<StepR>,
    {
        self.finalise(&out_ty)?;
        Ok(Step {
            out_ty,
            f: Arc::new(f),
        })
    }

    /// Preferred way of creating a eval step.
    ///
    /// This supplies the [`Value`] input but uses type inference on `O` to get the output type.
    pub fn eval_o<F, O>(self, f: F) -> Result<Step>
    where
        F: Fn(Value, Context) -> Result<(O, Environment)>,
        F: Func<Result<(O, Environment)>>,
        O: AsType + Into<Value>,
    {
        self.eval(O::as_type(), move |v, c| {
            f(v, c).map(|(x, e)| (Into::into(x), e))
        })
    }

    /// Carry out checks of the block's state.
    fn finalise(&self, _out_ty: &Type) -> Result<()> {
        if !self.flags.is_empty() {
            Err(Error::unused_flags(self.flags.iter()))
        } else if !self.args.is_empty() {
            Err(Error::unused_args(
                self.args.iter().map(|a| self.ag[a.idx()].tag()),
            ))
        } else {
            #[cfg(debug_assertions)]
            match &self.output_ty {
                Some(t) => debug_assert_eq!(
                    t, _out_ty,
                    "asserted output type should match finalisation type"
                ),
                None => (), // no assertion, no failure
            };

            Ok(())
        }
    }
}
