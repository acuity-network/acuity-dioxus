node_url := "ws://127.0.0.1:9944"

generate-runtime-api:
	subxt codegen --url {{node_url}} | rustfmt --edition 2021 > src/acuity_runtime.rs
