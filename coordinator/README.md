# Coordinator

Coordinator serves as modified Leader for evaluating multiple blocks.  Coordinator is a persistent webserver used to start the proving process blocks while recording the proofs along with benchmark statistics.

## Requests

To start the benchmarking process, you need to send a post request to the running endpoint.  It accepts the data formatted as a json.

### Fields

Subject to change, if any issues review the structs in the `input` module.

#### Required fields

- `start_block_number`: the first block to be included
- ``

#### Optional Fields

- `checkpoint_block_number`: The checkpoint block number, otherwise will be 0
- `terminate_on`: The conditions for termination.
- `proof_out`: If not provided, will not output the proofs.  

#### Terminate On

TODO: Describe the Termination settings

#### Proof Output

TODO: Describe the Proof Output settings

#### Benchmark Output

TODO: Describe the Benchmark Output settings.

### Examples

The example below proves blocks [1,10] using the RPC function listed in ZeroBin, outputs the proofs to a local directory where each proof will have a prefix of "test" (i.e. "test_1" for block 1, "test_2" for block 2, ...), and output the benchmark statistics locally to "test.csv".  The directories in which these files appear are established by the local environment.

```json
{
  "run_name": "run",
  "start_block_number": 1,
  "checkpoint_block_number": 1,
  "terminate_on": {
    "EndBlock": {"block_number": 10}
  },
  "block_source": {
    "ZeroBinRpc": {"rpc_url": "http://35.208.84.178:8545/"}
  },
  "proof_output": {
    "LocalDirectory": {"prefix": "test"}
  },
  "benchmark_output": {
    "LocalCsv": {"file_name": "test.csv"}
  }
}

```

An example not recording the proofs, and posting the results to a google cloud storage bucket.

```json
{
  "run_name": "run",
  "start_block_number": 1,
  "checkpoint_block_number": 1,
  "terminate_on": {
    "EndBlock": {"block_number": 10}
  },
  "block_source": {
    "ZeroBinRpc": {"rpc_url": "http://35.208.84.178:8545/"}
  },
  "benchmark_output": {
    "GoogleCloudStorageCsv": {"file_name": "test.csv", "bucket": "zkevm-csv"}
  }
}
```

In this example, we run the experiment for just one minute.  With `include_straddling` enabled, we
will also save the final block we prove even if it is after the elapsed seconds, but will not 
prove another block following that.

```json
{
  "start_block_number": 1,
  "checkpoint_block_number": 1,
  "terminate_on": {
    "ElapsedSeconds": {"num_seconds": 60, "include_straddling": true}
  },
  "block_source": {
    "ZeroBinRpc": {"rpc_url": "http://35.208.84.178:8545/"}
  },
  "benchmark_output": {
    "GoogleCloudStorageCsv": {"file_name": "test.csv", "bucket": "zkevm-csv"}
  }
}
```

```json
{
  "start_block_number": 1,
  "checkpoint_block_number": 1,
  "terminate_on": {
    "BlockGasUsed": {"until_gas_sum": 30000000}
  },
  "block_source": {
    "ZeroBinRpc": {"rpc_url": "http://35.208.84.178:8545/"}
  },
  "benchmark_output": {
    "GoogleCloudStorageCsv": {"file_name": "test.csv", "bucket": "zkevm-csv"}
  }
}
```

