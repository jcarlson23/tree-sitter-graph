// -*- coding: utf-8 -*-
// ------------------------------------------------------------------------------------------------
// Copyright © 2022, tree-sitter authors.
// Licensed under either of Apache License, Version 2.0, or MIT license, at your option.
// Please see the LICENSE-APACHE or LICENSE-MIT files in this distribution for license details.
// ------------------------------------------------------------------------------------------------

use thiserror::Error;
use tree_sitter::CaptureQuantifier;
use tree_sitter::CaptureQuantifier::One;
use tree_sitter::CaptureQuantifier::OneOrMore;
use tree_sitter::CaptureQuantifier::ZeroOrMore;
use tree_sitter::CaptureQuantifier::ZeroOrOne;
use tree_sitter::Query;

use crate::ast;
use crate::parser::FULL_MATCH;
use crate::variables::VariableError;
use crate::variables::VariableMap;
use crate::variables::Variables;
use crate::Context;
use crate::DisplayWithContext as _;
use crate::Location;

#[derive(Debug, Error)]
pub enum CheckError {
    #[error("Expected list value at {0}")]
    ExpectedListValue(Location),
    #[error("Expected optional value at {0}")]
    ExpectedOptionalValue(Location),
    #[error("Undefined syntax capture @{0} at {1}")]
    UndefinedSyntaxCapture(String, Location),
    #[error("{0}: {1}")]
    Variable(VariableError, String),
}

/// Checker context
struct CheckContext<'a> {
    ctx: &'a Context,
    locals: &'a mut dyn Variables<ExpressionResult>,
    file_query: &'a Query,
    stanza_index: usize,
    stanza_query: &'a Query,
}

//-----------------------------------------------------------------------------
// File

impl ast::File {
    pub fn check(&mut self, ctx: &Context) -> Result<(), CheckError> {
        let file_query = self.query.as_ref().unwrap();
        for (index, stanza) in self.stanzas.iter_mut().enumerate() {
            stanza.check(ctx, file_query, index)?;
        }
        Ok(())
    }
}

//-----------------------------------------------------------------------------
// Stanza

impl ast::Stanza {
    fn check(
        &mut self,
        ctx: &Context,
        file_query: &Query,
        stanza_index: usize,
    ) -> Result<(), CheckError> {
        let mut locals = VariableMap::new();
        let mut ctx = CheckContext {
            ctx,
            locals: &mut locals,
            file_query,
            stanza_index,
            stanza_query: &self.query,
        };
        self.full_match_file_capture_index =
            ctx.file_query.capture_index_for_name(FULL_MATCH).unwrap() as usize;
        for statement in &mut self.statements {
            statement.check(&mut ctx)?;
        }
        Ok(())
    }
}

//-----------------------------------------------------------------------------
// Statements

impl ast::Statement {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        match self {
            Self::DeclareImmutable(stmt) => stmt.check(ctx),
            Self::DeclareMutable(stmt) => stmt.check(ctx),
            Self::Assign(stmt) => stmt.check(ctx),
            Self::CreateGraphNode(stmt) => stmt.check(ctx),
            Self::AddGraphNodeAttribute(stmt) => stmt.check(ctx),
            Self::CreateEdge(stmt) => stmt.check(ctx),
            Self::AddEdgeAttribute(stmt) => stmt.check(ctx),
            Self::Scan(stmt) => stmt.check(ctx),
            Self::Print(stmt) => stmt.check(ctx),
            Self::If(stmt) => stmt.check(ctx),
            Self::ForIn(stmt) => stmt.check(ctx),
        }
    }
}

impl ast::DeclareImmutable {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        let value = self.value.check(ctx)?;
        self.variable.add_check(ctx, value, false)?;
        Ok(())
    }
}

impl ast::DeclareMutable {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        let value = self.value.check(ctx)?;
        self.variable.add_check(ctx, value, true)?;
        Ok(())
    }
}

impl ast::Assign {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        let value = self.value.check(ctx)?;
        self.variable.set_check(ctx, value)?;
        Ok(())
    }
}

impl ast::CreateGraphNode {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        self.node
            .add_check(ctx, ExpressionResult { quantifier: One }, false)?;
        Ok(())
    }
}

impl ast::AddGraphNodeAttribute {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        self.node.check(ctx)?;
        for attribute in &mut self.attributes {
            attribute.check(ctx)?;
        }
        Ok(())
    }
}

impl ast::CreateEdge {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        self.source.check(ctx)?;
        self.sink.check(ctx)?;
        Ok(())
    }
}

impl ast::AddEdgeAttribute {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        self.source.check(ctx)?;
        self.sink.check(ctx)?;
        for attribute in &mut self.attributes {
            attribute.check(ctx)?;
        }
        Ok(())
    }
}

impl ast::Scan {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        self.value.check(ctx)?;

        for arm in &mut self.arms {
            let mut arm_locals = VariableMap::new_child(ctx.locals);
            let mut arm_ctx = CheckContext {
                ctx: ctx.ctx,
                locals: &mut arm_locals,
                file_query: ctx.file_query,
                stanza_index: ctx.stanza_index,
                stanza_query: ctx.stanza_query,
            };

            for statement in &mut arm.statements {
                statement.check(&mut arm_ctx)?;
            }
        }
        Ok(())
    }
}

impl ast::Print {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        for value in &mut self.values {
            value.check(ctx)?;
        }
        Ok(())
    }
}

impl ast::If {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        for arm in &mut self.arms {
            for condition in &mut arm.conditions {
                condition.check(ctx)?;
            }

            let mut arm_locals = VariableMap::new_child(ctx.locals);
            let mut arm_ctx = CheckContext {
                ctx: ctx.ctx,
                locals: &mut arm_locals,
                file_query: ctx.file_query,
                stanza_index: ctx.stanza_index,
                stanza_query: ctx.stanza_query,
            };

            for statement in &mut arm.statements {
                statement.check(&mut arm_ctx)?;
            }
        }
        Ok(())
    }
}

impl ast::Condition {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        let captures = match self {
            Self::None(captures) => captures,
            Self::Some(captures) => captures,
        };

        for capture in captures {
            let result = capture.check(ctx)?;
            if result.quantifier != ZeroOrOne {
                return Err(CheckError::ExpectedOptionalValue(capture.location));
            }
        }
        Ok(())
    }
}

impl ast::ForIn {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        let capture = self.capture.check(ctx)?;
        if capture.quantifier != ZeroOrMore && capture.quantifier != OneOrMore {
            return Err(CheckError::ExpectedListValue(self.location));
        }

        let mut loop_locals = VariableMap::new_child(ctx.locals);
        let mut loop_ctx = CheckContext {
            ctx: ctx.ctx,
            locals: &mut loop_locals,
            file_query: ctx.file_query,
            stanza_index: ctx.stanza_index,
            stanza_query: ctx.stanza_query,
        };
        self.variable.add_check(&mut loop_ctx, capture, false)?;
        for statement in &mut self.statements {
            statement.check(&mut loop_ctx)?;
        }
        Ok(())
    }
}

//-----------------------------------------------------------------------------
// Expressions

/// Expression checking result
#[derive(Clone, Debug)]
struct ExpressionResult {
    quantifier: CaptureQuantifier,
}

impl ast::Expression {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        match self {
            Self::FalseLiteral => Ok(ExpressionResult { quantifier: One }),
            Self::NullLiteral => Ok(ExpressionResult { quantifier: One }),
            Self::TrueLiteral => Ok(ExpressionResult { quantifier: One }),
            Self::IntegerConstant(expr) => expr.check(ctx),
            Self::StringConstant(expr) => expr.check(ctx),
            Self::List(expr) => expr.check(ctx),
            Self::Set(expr) => expr.check(ctx),
            Self::Capture(expr) => expr.check(ctx),
            Self::Variable(expr) => expr.get_check(ctx),
            Self::Call(expr) => expr.check(ctx),
            Self::RegexCapture(expr) => expr.check(ctx),
        }
    }
}

impl ast::ScanExpression {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        match self {
            Self::StringConstant(expr) => expr.check(ctx),
            Self::Capture(expr) => expr.check(ctx),
            Self::Variable(expr) => expr.get_check(ctx),
            Self::RegexCapture(expr) => expr.check(ctx),
        }
    }
}

impl ast::IntegerConstant {
    fn check(&mut self, _ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        Ok(ExpressionResult { quantifier: One })
    }
}

impl ast::StringConstant {
    fn check(&mut self, _ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        Ok(ExpressionResult { quantifier: One })
    }
}

impl ast::ListComprehension {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        for element in &mut self.elements {
            element.check(ctx)?;
        }
        Ok(ExpressionResult {
            quantifier: ZeroOrMore,
        })
    }
}

impl ast::SetComprehension {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        for element in &mut self.elements {
            element.check(ctx)?;
        }
        Ok(ExpressionResult {
            quantifier: ZeroOrMore,
        })
    }
}

impl ast::Capture {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        let name = ctx.ctx.resolve(self.name);
        self.stanza_capture_index = ctx
            .stanza_query
            .capture_index_for_name(name)
            .ok_or_else(|| CheckError::UndefinedSyntaxCapture(name.to_string(), self.location))?
            as usize;
        self.file_capture_index = ctx.file_query.capture_index_for_name(name).unwrap() as usize;
        self.quantifier =
            ctx.file_query.capture_quantifiers(ctx.stanza_index)[self.file_capture_index];
        Ok(ExpressionResult {
            quantifier: self.quantifier,
        })
    }
}

impl ast::Call {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        for parameter in &mut self.parameters {
            parameter.check(ctx)?;
        }
        Ok(ExpressionResult {
            quantifier: One, // FIXME we don't really know
        })
    }
}

impl ast::RegexCapture {
    fn check(&mut self, _ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        Ok(ExpressionResult { quantifier: One })
    }
}

//-----------------------------------------------------------------------------
// Variables

impl ast::Variable {
    fn add_check(
        &mut self,
        ctx: &mut CheckContext,
        value: ExpressionResult,
        mutable: bool,
    ) -> Result<(), CheckError> {
        match self {
            Self::Unscoped(v) => v.add_check(ctx, value, mutable),
            Self::Scoped(v) => v.add_check(ctx, value, mutable),
        }
    }

    fn set_check(
        &mut self,
        ctx: &mut CheckContext,
        value: ExpressionResult,
    ) -> Result<(), CheckError> {
        match self {
            Self::Unscoped(v) => v.set_check(ctx, value),
            Self::Scoped(v) => v.set_check(ctx, value),
        }
    }

    fn get_check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        match self {
            Self::Unscoped(v) => v.get_check(ctx),
            Self::Scoped(v) => v.get_check(ctx),
        }
    }
}

impl ast::UnscopedVariable {
    fn add_check(
        &mut self,
        ctx: &mut CheckContext,
        value: ExpressionResult,
        mutable: bool,
    ) -> Result<(), CheckError> {
        ctx.locals
            .add(self.name, value, mutable)
            .map_err(|e| CheckError::Variable(e, format!("{}", self.name.display_with(ctx.ctx))))
    }

    fn set_check(
        &mut self,
        ctx: &mut CheckContext,
        value: ExpressionResult,
    ) -> Result<(), CheckError> {
        ctx.locals
            .set(self.name, value)
            .map_err(|e| CheckError::Variable(e, format!("{}", self.name.display_with(ctx.ctx))))
    }

    fn get_check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        // If the variable is not found, we return a default value for a possible global variable.
        let value = ctx
            .locals
            .get(&self.name)
            .cloned()
            .unwrap_or_else(|| ExpressionResult {
                quantifier: One, /* FIXME we don't really know */
            });
        Ok(value)
    }
}

impl ast::ScopedVariable {
    fn add_check(
        &mut self,
        ctx: &mut CheckContext,
        _value: ExpressionResult,
        _mutable: bool,
    ) -> Result<(), CheckError> {
        self.scope.check(ctx)?;
        Ok(())
    }

    fn set_check(
        &mut self,
        ctx: &mut CheckContext,
        _value: ExpressionResult,
    ) -> Result<(), CheckError> {
        self.scope.check(ctx)?;
        Ok(())
    }

    fn get_check(&mut self, ctx: &mut CheckContext) -> Result<ExpressionResult, CheckError> {
        self.scope.check(ctx)?;
        Ok(ExpressionResult {
            quantifier: One, // FIXME we don't really know
        })
    }
}

//-----------------------------------------------------------------------------
// Attributes

impl ast::Attribute {
    fn check(&mut self, ctx: &mut CheckContext) -> Result<(), CheckError> {
        self.value.check(ctx)?;
        Ok(())
    }
}
