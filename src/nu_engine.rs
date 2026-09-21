use std::{collections::BTreeMap, path::Path};

use nu_cmd_lang::create_default_context;
use nu_command::add_shell_command_context;
use nu_engine::eval_block_with_early_return;
use nu_parser::parse;
use nu_protocol::{
    Config, PipelineData, ShellError, Span, Value,
    debugger::WithoutDebug,
    engine::{Stack, StateWorkingSet},
};

pub(crate) fn evaluate(
    command: &str,
    working_dir: &Path,
    env: Option<&BTreeMap<String, String>>,
) -> Result<String, Box<ShellError>> {
    let mut engine_state = create_default_context();
    engine_state = add_shell_command_context(engine_state);
    for (name, value) in std::env::vars() {
        engine_state.add_env_var(name, Value::string(value, Span::unknown()));
    }
    engine_state.add_env_var(
        "PWD".to_owned(),
        Value::string(working_dir.to_string_lossy(), Span::unknown()),
    );

    let mut stack = Stack::new().capture_all().suppress_stdin();
    stack.set_cwd(working_dir)?;
    if let Some(env) = env {
        for (name, value) in env {
            stack.add_env_var(name.clone(), Value::string(value, Span::unknown()));
        }
    }

    let mut working_set = StateWorkingSet::new(&engine_state);
    let block = parse(&mut working_set, None, command.as_bytes(), false);
    if let Some(error) = working_set.parse_errors.first() {
        return Err(Box::new(ShellError::from(Box::new(
            nu_protocol::LabeledError::new(format!("Nushell parse error: {error}")),
        ))));
    }
    engine_state.merge_delta(working_set.render())?;

    let output = eval_block_with_early_return::<WithoutDebug>(
        &engine_state,
        &mut stack,
        &block,
        PipelineData::empty(),
    )?;
    let config: Config = (*stack.get_config(&engine_state)).clone();
    output.body.collect_string("\n", &config).map_err(Box::new)
}
