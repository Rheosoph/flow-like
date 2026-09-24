//! DataFusion 53 builds `named_struct`/`struct` children and `array_agg` list
//! items from bare data types, dropping the Arrow extension metadata that marks
//! a WKB column as geometry. These wrappers carry it into the nested field so
//! result decoding still sees geometry inside structs and lists.

use super::{EXTENSION_METADATA, EXTENSION_NAME};
use arrow_array::{Array, ArrayRef, ListArray, cast::AsArray};
use arrow_schema::{DataType, Field, FieldRef, Fields};
use datafusion::{
    common::ScalarValue,
    error::Result,
    execution::FunctionRegistry,
    logical_expr::{
        Accumulator, AggregateUDF, AggregateUDFImpl, ColumnarValue, Documentation, EmitTo,
        GroupsAccumulator, ReturnFieldArgs, ReversedUDAF, ScalarFunctionArgs, ScalarUDF,
        ScalarUDFImpl, Signature,
        function::{AccumulatorArgs, StateFieldsArgs},
        utils::AggregateOrderSensitivity,
    },
    prelude::SessionContext,
};
use std::{any::Any, collections::HashMap, sync::Arc};

pub(super) fn register_extension_preserving_nesting(context: &SessionContext) {
    for (name, named) in [("named_struct", true), ("struct", false)] {
        if let Ok(inner) = context.udf(name) {
            context.register_udf(ScalarUDF::from(ExtensionStruct { inner, named }));
        }
    }
    if let Ok(inner) = context.udaf("array_agg") {
        context.register_udaf(AggregateUDF::from(ExtensionArrayAgg { inner }));
    }
}

fn extension_metadata(field: &Field) -> HashMap<String, String> {
    field
        .metadata()
        .iter()
        .filter(|(key, _)| key.as_str() == EXTENSION_NAME || key.as_str() == EXTENSION_METADATA)
        .map(|(key, value)| (key.clone(), value.clone()))
        .collect()
}

fn carry_extension(target: &FieldRef, source: &Field) -> FieldRef {
    let extension = extension_metadata(source);
    if extension.is_empty() {
        return target.clone();
    }
    let mut metadata = target.metadata().clone();
    metadata.extend(extension);
    Arc::new(target.as_ref().clone().with_metadata(metadata))
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ExtensionStruct {
    inner: Arc<ScalarUDF>,
    named: bool,
}

impl ScalarUDFImpl for ExtensionStruct {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn aliases(&self) -> &[String] {
        self.inner.aliases()
    }
    fn signature(&self) -> &Signature {
        self.inner.signature()
    }
    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        self.inner.return_type(arg_types)
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let arg_fields = args.arg_fields;
        let field = self.inner.return_field_from_args(args)?;
        let DataType::Struct(children) = field.data_type() else {
            return Ok(field);
        };
        let (skip, step) = if self.named { (1, 2) } else { (0, 1) };
        let values = arg_fields.iter().skip(skip).step_by(step);
        let children: Fields = children
            .iter()
            .zip(values)
            .map(|(child, value)| carry_extension(child, value))
            .collect();
        Ok(Arc::new(
            field
                .as_ref()
                .clone()
                .with_data_type(DataType::Struct(children)),
        ))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        self.inner.invoke_with_args(args)
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.inner.documentation()
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct ExtensionArrayAgg {
    inner: Arc<AggregateUDF>,
}

/// The list item field the aggregate declared, when it carries an extension
/// the inner accumulators do not know about.
fn declared_item(args: &AccumulatorArgs) -> Option<FieldRef> {
    match args.return_field.data_type() {
        DataType::List(item) if !extension_metadata(item).is_empty() => Some(item.clone()),
        _ => None,
    }
}

fn relabeled(inner: Box<dyn Accumulator>, item: Option<FieldRef>) -> Box<dyn Accumulator> {
    match item {
        Some(item) => Box::new(RelabeledAccumulator { inner, item }),
        None => inner,
    }
}

fn relabel_list(list: &ListArray, item: &FieldRef) -> Result<ListArray> {
    let (_, offsets, values, nulls) = list.clone().into_parts();
    Ok(ListArray::try_new(item.clone(), offsets, values, nulls)?)
}

impl AggregateUDFImpl for ExtensionArrayAgg {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn name(&self) -> &str {
        self.inner.name()
    }
    fn aliases(&self) -> &[String] {
        self.inner.aliases()
    }
    fn signature(&self) -> &Signature {
        self.inner.signature()
    }
    fn return_type(&self, arg_types: &[DataType]) -> Result<DataType> {
        self.inner.return_type(arg_types)
    }
    fn return_field(&self, arg_fields: &[FieldRef]) -> Result<FieldRef> {
        let field = self.inner.return_field(arg_fields)?;
        match (field.data_type(), arg_fields.first()) {
            (DataType::List(item), Some(input)) => {
                Ok(Arc::new(field.as_ref().clone().with_data_type(
                    DataType::List(carry_extension(item, input)),
                )))
            }
            _ => Ok(field),
        }
    }
    fn state_fields(&self, args: StateFieldsArgs) -> Result<Vec<FieldRef>> {
        self.inner.state_fields(args)
    }
    fn order_sensitivity(&self) -> AggregateOrderSensitivity {
        self.inner.order_sensitivity()
    }
    fn with_beneficial_ordering(
        self: Arc<Self>,
        beneficial_ordering: bool,
    ) -> Result<Option<Arc<dyn AggregateUDFImpl>>> {
        Ok(self
            .inner
            .as_ref()
            .clone()
            .with_beneficial_ordering(beneficial_ordering)?
            .map(|inner| {
                Arc::new(Self {
                    inner: Arc::new(inner),
                }) as Arc<dyn AggregateUDFImpl>
            }))
    }
    fn accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        let item = declared_item(&args);
        Ok(relabeled(self.inner.accumulator(args)?, item))
    }
    fn create_sliding_accumulator(&self, args: AccumulatorArgs) -> Result<Box<dyn Accumulator>> {
        let item = declared_item(&args);
        Ok(relabeled(
            self.inner.create_sliding_accumulator(args)?,
            item,
        ))
    }
    fn groups_accumulator_supported(&self, args: AccumulatorArgs) -> bool {
        self.inner.groups_accumulator_supported(args)
    }
    fn create_groups_accumulator(
        &self,
        args: AccumulatorArgs,
    ) -> Result<Box<dyn GroupsAccumulator>> {
        let item = declared_item(&args);
        let inner = self.inner.create_groups_accumulator(args)?;
        Ok(match item {
            Some(item) => Box::new(RelabeledGroupsAccumulator { inner, item }),
            None => inner,
        })
    }
    fn reverse_expr(&self) -> ReversedUDAF {
        match self.inner.reverse_udf() {
            ReversedUDAF::Reversed(inner) => {
                ReversedUDAF::Reversed(Arc::new(AggregateUDF::from(Self { inner })))
            }
            other => other,
        }
    }
    fn supports_null_handling_clause(&self) -> bool {
        self.inner.supports_null_handling_clause()
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.inner.documentation()
    }
}

#[derive(Debug)]
struct RelabeledAccumulator {
    inner: Box<dyn Accumulator>,
    item: FieldRef,
}

impl Accumulator for RelabeledAccumulator {
    fn update_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.inner.update_batch(values)
    }
    fn evaluate(&mut self) -> Result<ScalarValue> {
        match self.inner.evaluate()? {
            ScalarValue::List(list) => Ok(ScalarValue::List(Arc::new(relabel_list(
                &list, &self.item,
            )?))),
            other => Ok(other),
        }
    }
    fn size(&self) -> usize {
        self.inner.size()
    }
    fn state(&mut self) -> Result<Vec<ScalarValue>> {
        self.inner.state()
    }
    fn merge_batch(&mut self, states: &[ArrayRef]) -> Result<()> {
        self.inner.merge_batch(states)
    }
    fn retract_batch(&mut self, values: &[ArrayRef]) -> Result<()> {
        self.inner.retract_batch(values)
    }
    fn supports_retract_batch(&self) -> bool {
        self.inner.supports_retract_batch()
    }
}

struct RelabeledGroupsAccumulator {
    inner: Box<dyn GroupsAccumulator>,
    item: FieldRef,
}

impl GroupsAccumulator for RelabeledGroupsAccumulator {
    fn update_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        opt_filter: Option<&arrow_array::BooleanArray>,
        total_num_groups: usize,
    ) -> Result<()> {
        self.inner
            .update_batch(values, group_indices, opt_filter, total_num_groups)
    }
    fn evaluate(&mut self, emit_to: EmitTo) -> Result<ArrayRef> {
        let array = self.inner.evaluate(emit_to)?;
        match array.data_type() {
            DataType::List(_) => Ok(Arc::new(relabel_list(array.as_list(), &self.item)?)),
            _ => Ok(array),
        }
    }
    fn state(&mut self, emit_to: EmitTo) -> Result<Vec<ArrayRef>> {
        self.inner.state(emit_to)
    }
    fn merge_batch(
        &mut self,
        values: &[ArrayRef],
        group_indices: &[usize],
        opt_filter: Option<&arrow_array::BooleanArray>,
        total_num_groups: usize,
    ) -> Result<()> {
        self.inner
            .merge_batch(values, group_indices, opt_filter, total_num_groups)
    }
    fn convert_to_state(
        &self,
        values: &[ArrayRef],
        opt_filter: Option<&arrow_array::BooleanArray>,
    ) -> Result<Vec<ArrayRef>> {
        self.inner.convert_to_state(values, opt_filter)
    }
    fn supports_convert_to_state(&self) -> bool {
        self.inner.supports_convert_to_state()
    }
    fn size(&self) -> usize {
        self.inner.size()
    }
}
