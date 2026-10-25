use datafusion::arrow::{
    array::{
        Array, ArrayRef, IntervalDayTimeBuilder, IntervalMonthDayNanoArray,
        IntervalYearMonthBuilder,
    },
    datatypes::{IntervalDayTimeType, IntervalYearMonthType},
    error::ArrowError,
};
use std::sync::Arc;

pub(crate) fn cast_interval_monthdaynano_to_yearmonth(
    interval_monthdaynano_array: &dyn Array,
) -> Result<ArrayRef, ArrowError> {
    let interval_monthdaynano_array = interval_monthdaynano_array
        .as_any()
        .downcast_ref::<IntervalMonthDayNanoArray>()
        .ok_or_else(|| {
            ArrowError::CastError("Failed to cast IntervalMonthDayNanoArray: Unable to downcast to IntervalMonthDayNanoArray".to_string())
        })?;

    let mut interval_yearmonth_builder =
        IntervalYearMonthBuilder::with_capacity(interval_monthdaynano_array.len());

    for value in interval_monthdaynano_array {
        match value {
            None => interval_yearmonth_builder.append_null(),
            Some(interval_monthdaynano_value) => {
                if interval_monthdaynano_value.days != 0
                    || interval_monthdaynano_value.nanoseconds != 0
                {
                    return Err(ArrowError::CastError(
                        "Failed to cast IntervalMonthDayNanoArray to IntervalYearMonthArray: Non-zero days or nanoseconds".to_string(),
                    ));
                }
                interval_yearmonth_builder.append_value(IntervalYearMonthType::make_value(
                    0,
                    interval_monthdaynano_value.months,
                ));
            }
        }
    }

    Ok(Arc::new(interval_yearmonth_builder.finish()))
}

#[allow(clippy::cast_possible_truncation)]
pub(crate) fn cast_interval_monthdaynano_to_daytime(
    interval_monthdaynano_array: &dyn Array,
) -> Result<ArrayRef, ArrowError> {
    let interval_monthdaynano_array = interval_monthdaynano_array
        .as_any()
        .downcast_ref::<IntervalMonthDayNanoArray>()
        .ok_or_else(|| ArrowError::CastError("Failed to cast IntervalMonthDayNanoArray: Unable to downcast to IntervalMonthDayNanoArray".to_string()))?;

    let mut interval_daytime_builder =
        IntervalDayTimeBuilder::with_capacity(interval_monthdaynano_array.len());

    for value in interval_monthdaynano_array {
        match value {
            None => interval_daytime_builder.append_null(),
            Some(interval_monthdaynano_value) => {
                if interval_monthdaynano_value.months != 0 {
                    return Err(
                        ArrowError::CastError("Failed to cast IntervalMonthDayNanoArray to IntervalDayTimeArray: Non-zero months".to_string()),
                    );
                }
                interval_daytime_builder.append_value(IntervalDayTimeType::make_value(
                    interval_monthdaynano_value.days,
                    (interval_monthdaynano_value.nanoseconds / 1_000_000) as i32,
                ));
            }
        }
    }
    Ok(Arc::new(interval_daytime_builder.finish()))
}

#[cfg(test)]
mod test {
    use datafusion::arrow::{
        array::{IntervalDayTimeArray, IntervalYearMonthArray, RecordBatch},
        datatypes::{
            DataType, Field, IntervalDayTime, IntervalMonthDayNano, IntervalUnit, Schema, SchemaRef,
        },
    };

    use crate::schema_cast::record_convert::try_cast_to;

    use super::*;

    fn input_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new(
                "interval_daytime",
                DataType::Interval(IntervalUnit::MonthDayNano),
                false,
            ),
            Field::new(
                "interval_monthday_nano",
                DataType::Interval(IntervalUnit::MonthDayNano),
                false,
            ),
            Field::new(
                "interval_yearmonth",
                DataType::Interval(IntervalUnit::MonthDayNano),
                false,
            ),
        ]))
    }

    fn output_schema() -> SchemaRef {
        Arc::new(Schema::new(vec![
            Field::new(
                "interval_daytime",
                DataType::Interval(IntervalUnit::DayTime),
                false,
            ),
            Field::new(
                "interval_monthday_nano",
                DataType::Interval(IntervalUnit::MonthDayNano),
                false,
            ),
            Field::new(
                "interval_yearmonth",
                DataType::Interval(IntervalUnit::YearMonth),
                false,
            ),
        ]))
    }

    fn batch_input() -> RecordBatch {
        let interval_daytime_array = IntervalMonthDayNanoArray::from(vec![
            IntervalMonthDayNano::new(0, 1, 1_000_000_000),
            IntervalMonthDayNano::new(0, 33, 0),
            IntervalMonthDayNano::new(0, 0, 43_200_000_000_000),
        ]);
        let interval_monthday_nano_array = IntervalMonthDayNanoArray::from(vec![
            IntervalMonthDayNano::new(1, 2, 1000),
            IntervalMonthDayNano::new(12, 1, 0),
            IntervalMonthDayNano::new(0, 0, 12 * 1000 * 1000),
        ]);
        let interval_yearmonth_array = IntervalMonthDayNanoArray::from(vec![
            IntervalMonthDayNano::new(2, 0, 0),
            IntervalMonthDayNano::new(25, 0, 0),
            IntervalMonthDayNano::new(-1, 0, 0),
        ]);

        RecordBatch::try_new(
            input_schema(),
            vec![
                Arc::new(interval_daytime_array),
                Arc::new(interval_monthday_nano_array),
                Arc::new(interval_yearmonth_array),
            ],
        )
        .expect("Failed to created arrow interval record batch")
    }

    fn batch_expected() -> RecordBatch {
        let interval_daytime_array = IntervalDayTimeArray::from(vec![
            IntervalDayTime::new(1, 1000),
            IntervalDayTime::new(33, 0),
            IntervalDayTime::new(0, 12 * 60 * 60 * 1000),
        ]);
        let interval_monthday_nano_array = IntervalMonthDayNanoArray::from(vec![
            IntervalMonthDayNano::new(1, 2, 1000),
            IntervalMonthDayNano::new(12, 1, 0),
            IntervalMonthDayNano::new(0, 0, 12 * 1000 * 1000),
        ]);
        let interval_yearmonth_array = IntervalYearMonthArray::from(vec![2, 25, -1]);

        RecordBatch::try_new(
            output_schema(),
            vec![
                Arc::new(interval_daytime_array),
                Arc::new(interval_monthday_nano_array),
                Arc::new(interval_yearmonth_array),
            ],
        )
        .expect("Failed to created arrow interval record batch")
    }

    #[test]
    fn test_cast_interval_with_schema() {
        let input_batch = batch_input();
        let expected = batch_expected();
        let actual = try_cast_to(input_batch, output_schema()).expect("cast should succeed");

        assert_eq!(actual, expected);
    }
}
