#!/bin/zsh
#
# 随机 fuzz。
#
# 为什么它不在 tools/verify.sh 的默认路径里：verify.sh 是一条以退出码为准的
# **确定性**门禁，同样的代码必须永远给同样的答案。随机 fuzz 不是——它这次
# 绿不代表下次绿，把它塞进去等于让门禁偶尔说谎。
#
# 分工：
#
#   fuzz 负责发现      →  这个脚本，随机种子，有时间预算，单独的 CI job
#   corpus 负责不复发  →  crates/yu-syntax/tests/corpus/，进 cargo test
#
# 它找到的每一个失败都会被写进 corpus 目录，然后由人最小化、改名、确认它在
# 修复前确实变红。见 crates/yu-syntax/tests/corpus/README.md。
#
# 用法:
#   tools/fuzz.sh            默认 30 秒
#   tools/fuzz.sh 300        跑 300 秒
#   YU_FUZZ_SEED=123 tools/fuzz.sh    复现某一次失败

set -euo pipefail

root="${0:A:h:h}"
cd "$root"

seconds="${1:-30}"

printf "\033[1m▸ yu-syntax fuzz（%s 秒）\033[0m\n" "$seconds"
YU_FUZZ_SECONDS="$seconds" cargo test --release -p yu-syntax --test differential \
    -- --ignored --nocapture fuzz_soak
