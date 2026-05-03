use directories::ProjectDirs;

/// Runs the CLI application logic.
///
/// # Errors
///
/// This function will return an error if the underlying operation fails.
pub fn run() -> anyhow::Result<()> {
    println!("Hello from tibber_cli!");

    if let Some(proj_dirs) = ProjectDirs::from("nl", "delfer", "tibber") {
        // This gives you the platform-specific cache directory
        let cache_dir = proj_dirs.cache_dir();

        // Example output:
        // Linux: /home/user/.cache/myclitool
        // macOS: /Users/user/Library/Caches/com.MyOrg.MyCliTool
        // Windows: C:\Users\user\AppData\Local\MyOrg\MyCliTool\cache
        println!("Cache directory is: {:?}", cache_dir);
    }

    Ok(())
}
