#!/usr/bin/env pwsh

#################
## CSHARP
# ./scripts/run-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=csharp --set cluster.size=m60

# ./scripts/run-find-and-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=csharp --set cluster.size=m60

# ./scripts/run-find-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=csharp --set cluster.size=m60

# ./scripts/run-find-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   -RunLabel "find_one_bench_1kb" `
#   -FindLimit 1 `
#   -CursorBatchSize 2 `
#   --set cluster.type=csharp --set cluster.size=m60

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=csharp --set cluster.size=m60

# ./scripts/run-write-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=csharp --set cluster.size=m60

#################
## RUST
# ./scripts/run-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=rust --set cluster.size=m60

# ./scripts/run-find-and-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=rust --set cluster.size=m60

# ./scripts/run-find-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=rust --set cluster.size=m60

# ./scripts/run-find-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   -RunLabel "find_one_bench_1kb" `
#   -FindLimit 1 `
#   -CursorBatchSize 2 `
#   --set cluster.type=rust --set cluster.size=m60

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=rust --set cluster.size=m60

# ./scripts/run-write-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   --set cluster.type=rust --set cluster.size=m60

####################### CUSTOM RUNS #######################

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 1800 `
#   -Workers @("128") `
#   --set cluster.type=rust --set cluster.size=m60

# ./scripts/run-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 600 `
#   -Workers @("64") `
#   --set cluster.type=csharp --set cluster.size=m60

# $size1m = $(1024 * 1024)

# ./scripts/run-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 600 `
#   -DocSize $size1m `
#   -RunLabel "update_bench_1mb" `
#   -Workers @("8") `
#   -PreloadCount 128 `
#   --set cluster.type=csharp --set cluster.size=m60

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/onebox.secret `
#   -Driver benchly `
#   -Duration 30 `
#   -Workers @("128") `
#   --set cluster.type=rust-onebox --set cluster.size=m60

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.rust.secret `
#   -Driver benchly `
#   -Duration 30 `
#   -Workers @("128") `
#   --set cluster.type=rust-prod --set cluster.size=m60

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 30 `
#   -Workers @("128") `
#   --set cluster.type=csharp-prod --set cluster.size=m60

# ./scripts/run-read-bench.ps1 `
#   -MongoDbUrlFile ./secrets/onebox.secret `
#   -Driver benchly `
#   -Duration 30 `
#   -Workers @("256") `
#   --set cluster.type=csharp-onebox --set cluster.size=m60


# $size1m = $(1024 * 1024)
# $size256kb = $(256 * 1024)

# ./scripts/run-update-bench.ps1 `
#   -MongoDbUrlFile ./secrets/m60.csharp.secret `
#   -Driver benchly `
#   -Duration 90000 `
#   -DocSize $size256kb `
#   -RunLabel "update_bench_256kb" `
#   -Workers @("256") `
#   --set cluster.type=csharp --set cluster.size=m60

./scripts/run-update-bench.ps1 `
  -MongoDbUrlFile ./secrets/onebox.secret `
  -Driver benchly `
  -Duration 30 `
  -Workers @("10") `
  --set cluster.type=csharp --set cluster.size=m60