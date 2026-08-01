use super::*;

pub(super) fn corpus(command: CorpusCommand) -> Result<(), Box<dyn Error>> {
    let compiler = DockerCompiler::default();
    match command {
        CorpusCommand::Doctor => {
            compiler.verify_available()?;
            println!("Docker compiler boundary is available");
        }
        CorpusCommand::Matrix(arguments) => {
            let matrix_text = fs::read_to_string(arguments.matrix)?;
            let matrix: CompilerMatrix = toml::from_str(&matrix_text)?;
            fs::create_dir_all(&arguments.output)?;
            fs::create_dir_all(&arguments.artifacts)?;
            for variant in matrix.expand() {
                if !variant
                    .id
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric() || "._-".contains(character))
                {
                    return Err(format!("unsafe matrix variant ID {:?}", variant.id).into());
                }
                let output = arguments.output.join(&variant.id);
                fs::create_dir_all(&output)?;
                let request = BuildRequest {
                    toolchain: variant.toolchain,
                    source_dir: arguments.source.clone(),
                    output_dir: output,
                    arguments: variant.arguments,
                    target: arguments.target.clone(),
                    limits: DockerLimits {
                        timeout_seconds: arguments.timeout_seconds,
                        ..DockerLimits::default()
                    },
                };
                let artifact = compiler.compile(&request)?;
                artifact.write_json(&arguments.artifacts.join(format!("{}.json", variant.id)))?;
                if !artifact.succeeded() {
                    return Err(format!(
                        "matrix variant {:?} exited with status {}",
                        variant.id, artifact.exit_code
                    )
                    .into());
                }
            }
        }
        CorpusCommand::Compare(arguments) => {
            let observations = arguments
                .observations
                .iter()
                .map(|path| {
                    let value = serde_json::from_slice(&fs::read(path)?)?;
                    Ok(NamedObservation {
                        name: path.display().to_string(),
                        value,
                    })
                })
                .collect::<Result<Vec<_>, Box<dyn Error>>>()?;
            let comparisons = compare_observations(&observations, &arguments.pointers);
            println!("{}", serde_json::to_string_pretty(&comparisons)?);
            if comparisons
                .iter()
                .any(|comparison| !comparison.equivalent())
            {
                return Err("selected observations diverged".into());
            }
        }
        CorpusCommand::Run(arguments) => run_corpus_suite(&arguments)?,
        CorpusCommand::Reduce(arguments) => reduce_corpus_case(&compiler, &arguments)?,
        CorpusCommand::Build(arguments) => {
            let spec_text = fs::read_to_string(&arguments.toolchain)?;
            let toolchain: ToolchainSpec = toml::from_str(&spec_text)?;
            let request = BuildRequest {
                toolchain,
                source_dir: arguments.source,
                output_dir: arguments.output,
                arguments: arguments.arguments,
                target: arguments.target,
                limits: DockerLimits {
                    timeout_seconds: arguments.timeout_seconds,
                    ..DockerLimits::default()
                },
            };
            if arguments.dry_run {
                println!(
                    "{}",
                    serde_json::to_string_pretty(&compiler.command(&request)?)?
                );
            } else {
                let artifact = compiler.compile(&request)?;
                artifact.write_json(&arguments.artifact)?;
                if !artifact.succeeded() {
                    return Err(format!(
                        "container compiler exited with status {}",
                        artifact.exit_code
                    )
                    .into());
                }
            }
        }
    }
    Ok(())
}

#[derive(Clone, Debug, Serialize)]
struct ReductionEvaluation {
    id: u64,
    candidate: ReductionCandidate,
    build: BuildArtifact,
    #[serde(skip_serializing_if = "Option::is_none")]
    run: Option<RunResult>,
    #[serde(skip_serializing_if = "Option::is_none")]
    error: Option<String>,
    discrepancy: bool,
}

#[derive(Debug, Serialize)]
struct CorpusReductionArtifact {
    schema: &'static str,
    target: TargetId,
    seeded_expected: u32,
    reduction: CaseReductionResult,
    evaluations: Vec<ReductionEvaluation>,
    final_repeat_evaluations: [u64; 2],
    final_reproducible: bool,
    result: &'static str,
}

fn reduce_corpus_case(
    compiler: &DockerCompiler,
    arguments: &CorpusReduceArgs,
) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let toolchain: ToolchainSpec = toml::from_str(&fs::read_to_string(&arguments.toolchain)?)?;
    let original = ReductionCandidate {
        source: arguments.source_items.clone(),
        flags: arguments.flag_items.clone(),
        inputs: arguments.input_items.clone(),
    };
    fs::create_dir_all(&arguments.output)?;
    let mut evaluations = Vec::new();
    let mut next_id = 0_u64;

    let mut evaluate = |candidate: &ReductionCandidate| -> Result<bool, Box<dyn Error>> {
        let id = next_id;
        next_id = next_id.saturating_add(1);
        let evaluation_root = arguments.output.join(format!("evaluation-{id:04}"));
        let source_dir = evaluation_root.join("source");
        let output_dir = evaluation_root.join("output");
        fs::create_dir_all(&source_dir)?;
        fs::create_dir_all(&output_dir)?;
        copy_directory_contents(&arguments.source, &source_dir)?;

        let mut header = String::from("/* Generated deterministic reduction candidate. */\n");
        for fragment in &candidate.source {
            header.push_str(fragment);
            header.push('\n');
        }
        header.push_str("#define REMU_INPUT_SUM (0u");
        for input in &candidate.inputs {
            write!(header, " + {input}u")?;
        }
        header.push_str(")\n");
        fs::write(source_dir.join("candidate.h"), header)?;

        let mut compiler_arguments = arguments.arguments.clone();
        compiler_arguments.extend(candidate.flags.iter().cloned());
        let request = BuildRequest {
            toolchain: toolchain.clone(),
            source_dir,
            output_dir: output_dir.clone(),
            arguments: compiler_arguments,
            target: arguments.target.clone(),
            limits: DockerLimits::default(),
        };
        let build = compiler.compile(&request)?;
        build.write_json(&evaluation_root.join("build.json"))?;

        let mut run = None;
        let mut error = None;
        if build.succeeded() {
            match fs::read(output_dir.join("smoke.elf"))
                .map_err(|problem| problem.to_string())
                .and_then(|bytes| {
                    FirmwareImage::parse(&bytes).map_err(|problem| problem.to_string())
                })
                .and_then(|image| {
                    run_loaded(
                        target,
                        &image,
                        RunLimits {
                            instructions: Some(arguments.max_instructions),
                            deadline: None,
                        },
                        &[],
                        None,
                    )
                    .map_err(|problem| problem.to_string())
                }) {
                Ok(result) => {
                    fs::write(
                        evaluation_root.join("run.json"),
                        serde_json::to_vec_pretty(&result)?,
                    )?;
                    run = Some(result);
                }
                Err(problem) => error = Some(problem),
            }
        } else {
            error = Some(format!("compiler exited with status {}", build.exit_code));
        }
        let discrepancy = run
            .as_ref()
            .is_some_and(|result| result.exit_code != Some(arguments.seed_expected));
        evaluations.push(ReductionEvaluation {
            id,
            candidate: candidate.clone(),
            build,
            run,
            error,
            discrepancy,
        });
        Ok(discrepancy)
    };

    if !evaluate(&original)? {
        return Err("initial reduction case does not reproduce the seeded discrepancy".into());
    }
    let reduction = reduce_case(original, &mut evaluate)?;
    if !evaluate(&reduction.minimized)? || !evaluate(&reduction.minimized)? {
        return Err("minimized discrepancy did not reproduce twice".into());
    }
    let second = evaluations.last().expect("repeat evaluation exists");
    let first = &evaluations[evaluations.len() - 2];
    let final_reproducible = first.run == second.run
        && first.build.inputs == second.build.inputs
        && first.build.outputs == second.build.outputs;
    if !final_reproducible {
        return Err("minimized build or run artifacts are not reproducible".into());
    }
    let final_repeat_evaluations = [first.id, second.id];
    let artifact = CorpusReductionArtifact {
        schema: "remu.corpus-reduction.v1",
        target,
        seeded_expected: arguments.seed_expected,
        reduction,
        evaluations,
        final_repeat_evaluations,
        final_reproducible,
        result: "pass",
    };
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&artifact)?)?;
    println!(
        "reduced seeded discrepancy for {target}; artifact: {}",
        arguments.artifact.display()
    );
    Ok(())
}

fn copy_directory_contents(source: &Path, destination: &Path) -> Result<(), Box<dyn Error>> {
    for entry in fs::read_dir(source)? {
        let entry = entry?;
        let file_type = entry.file_type()?;
        let output = destination.join(entry.file_name());
        if file_type.is_dir() {
            fs::create_dir_all(&output)?;
            copy_directory_contents(&entry.path(), &output)?;
        } else if file_type.is_file() {
            fs::copy(entry.path(), output)?;
        } else {
            return Err(format!(
                "reduction source contains unsupported entry {}",
                entry.path().display()
            )
            .into());
        }
    }
    Ok(())
}

#[derive(Debug)]
struct ExpectedCase {
    name: String,
    category: String,
    signature: String,
    expected: u32,
    inspiration: String,
}

#[derive(Debug, Serialize)]
struct SuiteFailure {
    case_id: String,
    name: String,
    category: String,
    signature: String,
    inspiration: String,
    expected: u32,
    actual: Option<u32>,
    reason: String,
    instructions: Option<u64>,
}

#[derive(Debug, Serialize)]
struct SuiteArtifact {
    schema: &'static str,
    target: TargetId,
    input: String,
    manifest: String,
    total: usize,
    passed: usize,
    failed: usize,
    failures: Vec<SuiteFailure>,
}

fn run_corpus_suite(arguments: &CorpusRunArgs) -> Result<(), Box<dyn Error>> {
    let target = arguments.target.parse::<TargetId>()?;
    let expected = read_expected_manifest(&arguments.manifest)?;
    let mut failures = Vec::new();

    for (case_id, case) in &expected {
        let path = arguments.input.join(format!("{case_id}.elf"));
        let outcome = fs::read(&path)
            .map_err(|error| format!("{}: {error}", path.display()))
            .and_then(|bytes| {
                FirmwareImage::parse(&bytes)
                    .map_err(|error| error.to_string())
                    .and_then(|image| {
                        run_loaded(
                            target,
                            &image,
                            RunLimits {
                                instructions: Some(arguments.max_instructions),
                                deadline: None,
                            },
                            &[],
                            None,
                        )
                        .map_err(|error| error.to_string())
                    })
            });
        match outcome {
            Ok(result) if result.exit_code == Some(case.expected) => {}
            Ok(result) => failures.push(SuiteFailure {
                case_id: case_id.clone(),
                name: case.name.clone(),
                category: case.category.clone(),
                signature: case.signature.clone(),
                inspiration: case.inspiration.clone(),
                expected: case.expected,
                actual: result.exit_code,
                reason: format!("{:?}", result.reason),
                instructions: Some(result.stats.instructions),
            }),
            Err(error) => failures.push(SuiteFailure {
                case_id: case_id.clone(),
                name: case.name.clone(),
                category: case.category.clone(),
                signature: case.signature.clone(),
                inspiration: case.inspiration.clone(),
                expected: case.expected,
                actual: None,
                reason: error,
                instructions: None,
            }),
        }
    }

    let total = expected.len();
    let artifact = SuiteArtifact {
        schema: "remu.corpus-suite.v1",
        target,
        input: normalized_display_path(&arguments.input),
        manifest: normalized_display_path(&arguments.manifest),
        total,
        passed: total - failures.len(),
        failed: failures.len(),
        failures,
    };
    if let Some(parent) = arguments.artifact.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(&arguments.artifact, serde_json::to_vec_pretty(&artifact)?)?;
    println!(
        "{}: {}/{} cases passed; artifact: {}",
        target,
        artifact.passed,
        artifact.total,
        arguments.artifact.display()
    );
    if artifact.failed != 0 {
        return Err(format!("{} corpus cases failed", artifact.failed).into());
    }
    Ok(())
}

fn read_expected_manifest(path: &Path) -> Result<BTreeMap<String, ExpectedCase>, Box<dyn Error>> {
    let contents = fs::read_to_string(path)?;
    let mut expected = BTreeMap::new();
    for (line_number, line) in contents.lines().enumerate() {
        if line_number == 0
            && line == "case_id\tname\tcategory\tsignature\texpected_hex\tinspiration"
        {
            continue;
        }
        let fields = line.split('\t').collect::<Vec<_>>();
        if fields.len() != 6 {
            return Err(format!(
                "{}:{}: expected six TSV fields",
                path.display(),
                line_number + 1
            )
            .into());
        }
        let value = fields[4]
            .strip_prefix("0x")
            .ok_or_else(|| format!("{}:{}: expected 0x result", path.display(), line_number + 1))?;
        let parsed = u32::from_str_radix(value, 16)?;
        if expected
            .insert(
                fields[0].to_owned(),
                ExpectedCase {
                    name: fields[1].to_owned(),
                    category: fields[2].to_owned(),
                    signature: fields[3].to_owned(),
                    expected: parsed,
                    inspiration: fields[5].to_owned(),
                },
            )
            .is_some()
        {
            return Err(format!("duplicate case ID {:?}", fields[0]).into());
        }
    }
    if expected.len() != 1_000 {
        return Err(format!("expected exactly 1000 cases, found {}", expected.len()).into());
    }
    Ok(expected)
}

fn normalized_display_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
