//! 本地版本不提供云端 handoff；保留空视图类型以维持 transcript 的类型边界。

mod block;

pub(crate) use block::{TuiHandoffBlock, TuiHandoffBlockEvent, init};
