use crate::commands::WholeStreamCommand;
use crate::data::base::shape::Shapes;
use crate::prelude::*;
use futures_util::pin_mut;
use futures_util::stream::StreamExt;
use log::trace;
use nu_errors::ShellError;
use nu_protocol::{
    did_you_mean, ColumnPath, ReturnSuccess, ReturnValue, Signature, SyntaxShape, UntaggedValue,
    Value,
};
use nu_source::{span_for_spanned_list, PrettyDebug};
use nu_value_ext::get_data_by_column_path;

pub struct Get;

#[derive(Deserialize)]
pub struct GetArgs {
    rest: Vec<ColumnPath>,
}

impl WholeStreamCommand for Get {
    fn name(&self) -> &str {
        "get"
    }

    fn signature(&self) -> Signature {
        Signature::build("get").rest(
            SyntaxShape::ColumnPath,
            "optionally return additional data by path",
        )
    }

    fn usage(&self) -> &str {
        "Open given cells as text."
    }

    fn run(
        &self,
        args: CommandArgs,
        registry: &CommandRegistry,
    ) -> Result<OutputStream, ShellError> {
        args.process(registry, get)?.run()
    }
}

pub fn get_column_path(path: &ColumnPath, obj: &Value) -> Result<Value, ShellError> {
    let fields = path.clone();

    get_data_by_column_path(
        obj,
        path,
        Box::new(move |(obj_source, column_path_tried, error)| {
            if let UntaggedValue::Table(rows) = &obj_source.value {
                let total = rows.len();
                let end_tag = match fields
                    .members()
                    .iter()
                    .nth_back(if fields.members().len() > 2 { 1 } else { 0 })
                {
                    Some(last_field) => last_field.span,
                    None => column_path_tried.span,
                };

                let primary_label = format!(
                    "There isn't a row indexed at {}",
                    column_path_tried.display()
                );

                let secondary_label = if total == 1 {
                    "The table only has 1 row".to_owned()
                } else {
                    format!("The table only has {} rows (0 to {})", total, total - 1)
                };

                return ShellError::labeled_error_with_secondary(
                    "Row not found",
                    primary_label,
                    column_path_tried.span,
                    secondary_label,
                    end_tag,
                );
            }

            if let Some(suggestions) = did_you_mean(&obj_source, column_path_tried) {
                return ShellError::labeled_error(
                    "Unknown column",
                    format!("did you mean '{}'?", suggestions[0].1),
                    span_for_spanned_list(fields.members().iter().map(|p| p.span)),
                );
            }

            error
        }),
    )
}

pub fn get(
    GetArgs { rest: mut fields }: GetArgs,
    RunnableContext { input, .. }: RunnableContext,
) -> Result<OutputStream, ShellError> {
    if fields.is_empty() {
        let stream = async_stream! {
            let values = input.values;
            pin_mut!(values);

            let mut shapes = Shapes::new();
            let mut index = 0;

            while let Some(row) = values.next().await {
                shapes.add(&row, index);
                index += 1;
            }

            for row in shapes.to_values() {
                yield ReturnSuccess::value(row);
            }
        };

        let stream: BoxStream<'static, ReturnValue> = stream.boxed();

        Ok(stream.to_output_stream())
    } else {
        let member = fields.remove(0);
        trace!("get {:?} {:?}", member, fields);
        let stream = input
            .values
            .map(move |item| {
                let mut result = VecDeque::new();

                let member = vec![member.clone()];

                let column_paths = vec![&member, &fields]
                    .into_iter()
                    .flatten()
                    .collect::<Vec<&ColumnPath>>();

                for path in column_paths {
                    let res = get_column_path(&path, &item);

                    match res {
                        Ok(got) => match got {
                            Value {
                                value: UntaggedValue::Table(rows),
                                ..
                            } => {
                                for item in rows {
                                    result.push_back(ReturnSuccess::value(item.clone()));
                                }
                            }
                            other => result.push_back(ReturnSuccess::value(other.clone())),
                        },
                        Err(reason) => result.push_back(ReturnSuccess::value(
                            UntaggedValue::Error(reason).into_untagged_value(),
                        )),
                    }
                }

                result
            })
            .flatten();

        Ok(stream.to_output_stream())
    }
}
