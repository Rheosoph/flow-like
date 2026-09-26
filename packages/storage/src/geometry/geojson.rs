//! Geometry text for SQL: `ST_AsGeoJSON(geometry)` and `ST_GeomFromGeoJSON(text)`,
//! which geodatafusion 0.4 does not provide, and `ST_AsText` as plain WKT text.

use super::{decode_column, geometry_field, geometry_input_wkb, is_geometry_field, wkb_result};
use arrow_array::{StringArray, cast::AsArray};
use arrow_schema::{DataType, Field, FieldRef};
use datafusion::{
    common::ScalarValue,
    error::{DataFusionError, Result},
    execution::FunctionRegistry,
    logical_expr::{
        ColumnarValue, Documentation, ReturnFieldArgs, ScalarFunctionArgs, ScalarUDF,
        ScalarUDFImpl, Signature, Volatility,
    },
    prelude::SessionContext,
};
use flow_like_types::Value;
use std::{any::Any, sync::Arc};

pub(super) fn register_text_functions(context: &SessionContext) {
    context.register_udf(ScalarUDF::from(AsGeoJson::default()));
    context.register_udf(ScalarUDF::from(GeomFromGeoJson::default()));
    if let Ok(inner) = context.udf("st_astext") {
        context.register_udf(ScalarUDF::from(PlainText { inner }));
    }
}

/// geodatafusion 0.4 tags the WKT from `ST_AsText` as a `geoarrow.wkt` geometry,
/// so result readers would decode it back into GeoJSON instead of showing the text.
#[derive(Debug, PartialEq, Eq, Hash)]
struct PlainText {
    inner: Arc<ScalarUDF>,
}

impl ScalarUDFImpl for PlainText {
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
        let field = self.inner.return_field_from_args(args)?;
        Ok(Arc::new(Field::new(
            field.name(),
            field.data_type().clone(),
            field.is_nullable(),
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        self.inner.invoke_with_args(args)
    }
    fn documentation(&self) -> Option<&Documentation> {
        self.inner.documentation()
    }
}

fn execution_error(function: &str, error: impl std::fmt::Display) -> DataFusionError {
    DataFusionError::Execution(format!("{function}: {error}"))
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct AsGeoJson {
    signature: Signature,
}

impl Default for AsGeoJson {
    fn default() -> Self {
        Self {
            signature: Signature::any(1, Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for AsGeoJson {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn name(&self) -> &str {
        "st_asgeojson"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Utf8)
    }
    fn return_field_from_args(&self, args: ReturnFieldArgs) -> Result<FieldRef> {
        let input = &args.arg_fields[0];
        if !is_geometry_field(input) && input.data_type() != &DataType::Null {
            return Err(DataFusionError::Plan(format!(
                "ST_AsGeoJSON takes a geometry, but '{}' is {} without GeoArrow geometry metadata; convert text with ST_GeomFromGeoJSON(text) or flow_geomfromtext(wkt) first",
                input.name(),
                input.data_type()
            )));
        }
        Ok(Arc::new(Field::new(
            self.name(),
            DataType::Utf8,
            input.is_nullable(),
        )))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let scalar = matches!(args.args[0], ColumnarValue::Scalar(_));
        let field = args.arg_fields[0].clone();
        let array = ColumnarValue::values_to_arrays(&args.args)?.remove(0);
        let text: StringArray = if field.data_type() == &DataType::Null {
            StringArray::new_null(array.len())
        } else {
            decode_column(array.as_ref(), &field)
                .map_err(|error| execution_error("ST_AsGeoJSON", error))?
                .iter()
                .map(|geometry| (!geometry.is_null()).then(|| geometry.to_string()))
                .collect()
        };
        if scalar {
            return Ok(ColumnarValue::Scalar(ScalarValue::Utf8(
                text.iter().next().flatten().map(str::to_owned),
            )));
        }
        Ok(ColumnarValue::Array(Arc::new(text)))
    }
}

#[derive(Debug, PartialEq, Eq, Hash)]
struct GeomFromGeoJson {
    signature: Signature,
}

impl Default for GeomFromGeoJson {
    fn default() -> Self {
        Self {
            signature: Signature::string(1, Volatility::Immutable),
        }
    }
}

impl ScalarUDFImpl for GeomFromGeoJson {
    fn as_any(&self) -> &dyn Any {
        self
    }
    fn name(&self) -> &str {
        "st_geomfromgeojson"
    }
    fn signature(&self) -> &Signature {
        &self.signature
    }
    fn return_type(&self, _: &[DataType]) -> Result<DataType> {
        Ok(DataType::Binary)
    }
    fn return_field_from_args(&self, _: ReturnFieldArgs) -> Result<FieldRef> {
        Ok(Arc::new(geometry_field(self.name(), true)))
    }
    fn invoke_with_args(&self, args: ScalarFunctionArgs) -> Result<ColumnarValue> {
        let scalar = matches!(args.args[0], ColumnarValue::Scalar(_));
        let array = ColumnarValue::values_to_arrays(&args.args)?.remove(0);
        let text = arrow::compute::cast(&array, &DataType::Utf8)?;
        let values = text
            .as_string::<i32>()
            .iter()
            .map(|text| match text {
                Some(text) => geometry_input_wkb(&Value::String(text.to_owned())),
                None => Ok(None),
            })
            .collect::<flow_like_types::Result<Vec<_>>>()
            .map_err(|error| execution_error("ST_GeomFromGeoJSON", error))?;
        Ok(wkb_result(scalar, values))
    }
}
