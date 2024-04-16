
# Geth Versus Jerigon

Comparing Geth vs **J**erigon

## ZeroBin Tracer

The [Jerigon/feat-zero](https://github.com/0xPolygonZero/erigon/tree/feat/zero) branch we use seems like it comes with the `ZeroTracer` tracer preinstalled.  Unless moved over, it will only be in the Polyon's forked Erigon, **J**erigon.

## API Calls

Here we compare the API calls used in ZeroBin and the return differences.

### debug_traceBlockByNumber

There's some key values missing returned from the Geth's `debug_traceBlockByNumber` versus Jerigon.

| Value           | Jerigon | Geth   |
|-----------------|---------|--------|
|from             |X        |X       |
|to               |X        |X       |
|gas              |X        |X       |
|transaction_value|X        |        |
|gasUsed          |X        |X       |
|input            |X        |X       |
|output           |X        |        |
|error            |X        |        |
|calls            |X        |        |
|revertReason     |X        |        |
|type             |X        |X       |

#### Missing Values

| Value | Description |
|-------|-------------|
| transaction_value |  The actual value per gas deducted from the sender's account. |
| output | The return value of the call, encoded as a hexadecimal string. |
| error  | An error if the execution failed. |
| calls  | A list of sub-calls made by the contract during the call, each represented as a nested call frame object. |
| reverseReason | The reason why the transaction was reverted, returned by the smart contract if any. |



### eth_getBlockByNumber

The `eth_getBlockByNumber` api returns are fairly similar

| Value           | Jerigon | Geth   |
|-----------------|---------|--------|
| number          | X       | X      |
| hash            | X       | X      |
| parentHash      | X       | X      |
| nonce           | X       | X      |
| difficulty      | X       | X      |
| totalDifficulty | X       | X      |
| logsBloom       | X       | X      |
| sha3uncles      | X       | X      |
| extraData       | X       | X      |
| timestamp       | X       | X      |
| size            | X       | X      |
| miner           | X       | X      |
| transactionsRoot| X       | X      |
| stateRoot       | X       | X      |
| receiptsRoot    | X       | X      |
| uncles          | X       | X      |
| transactions*   | X       | X      |
| gasUsed         | X       | X      |
| gasLimit        | X       | X      |
| mixHash         | X       | X      |
| baseFeePerGas   | X       | X      |

### eth_chainId

Just returns chain id, should be the same api between both Geth and Jerigon.
