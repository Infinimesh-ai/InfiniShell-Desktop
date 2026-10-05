#[cfg(unix)]
use std::sync::Arc;

use super::*;
#[cfg(unix)]
use crate::terminal::model::session::SessionInfo;
#[cfg(unix)]
use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
#[cfg(unix)]
use crate::terminal::shell::ShellType;

#[cfg(unix)]
#[test]
fn open_file_command_expands_unix_session_home_with_spaces() {
    let session = Session::new(
        SessionInfo::new_for_test()
            .with_shell_type(ShellType::Bash)
            .with_home_dir("/srv/remote user".to_owned()),
        Arc::new(TestCommandExecutor::default()),
    );

    let (path, line_col) =
        open_file_command_path(&session, "/tmp", "~/project/file\\ name.txt:4:2");

    assert_eq!(
        path,
        PathBuf::from("/srv/remote user/project/file name.txt")
    );
    assert_eq!(
        line_col,
        Some(LineAndColumnArg {
            line_num: 4,
            column_num: Some(2),
        })
    );
}

#[cfg(windows)]
mod windows {
    use std::sync::Arc;

    use super::*;
    use crate::terminal::ShellLaunchData;
    use crate::terminal::model::session::SessionInfo;
    use crate::terminal::model::session::command_executor::testing::TestCommandExecutor;
    use crate::terminal::shell::ShellType;

    fn wsl_session() -> Session {
        Session::new(
            SessionInfo::new_for_test()
                .with_shell_type(ShellType::Bash)
                .with_home_dir("/home/ubuntu".to_owned()),
            Arc::new(TestCommandExecutor::default()),
        )
        .with_shell_launch_data(ShellLaunchData::WSL {
            distro: "Ubuntu".to_owned(),
        })
    }

    #[test]
    fn open_file_command_converts_wsl_paths_to_host_paths() {
        let session = wsl_session();
        let cases = [
            (
                "/home/ubuntu",
                "subdir/test.txt",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\test.txt",
                None,
            ),
            (
                "/home/ubuntu/project",
                "../test.txt",
                r"\\WSL$\Ubuntu\home\ubuntu\test.txt",
                None,
            ),
            (
                "/home/ubuntu",
                "subdir/file\\ name.txt",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\file name.txt",
                None,
            ),
            (
                "/home/ubuntu",
                "subdir/test.txt:4:2",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\test.txt",
                Some(LineAndColumnArg {
                    line_num: 4,
                    column_num: Some(2),
                }),
            ),
            (
                "/tmp",
                "~/subdir/file\\ name.txt:4:2",
                r"\\WSL$\Ubuntu\home\ubuntu\subdir\file name.txt",
                Some(LineAndColumnArg {
                    line_num: 4,
                    column_num: Some(2),
                }),
            ),
        ];

        for (current_dir, raw_arg, expected_path, expected_line_col) in cases {
            let (path, line_col) = open_file_command_path(&session, current_dir, raw_arg);

            assert_eq!(path, PathBuf::from(expected_path));
            assert_eq!(line_col, expected_line_col);
        }
    }
}
