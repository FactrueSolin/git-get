//! git-get: 从 GitHub 仓库下载指定子目录或整个仓库的命令行工具
//!
//! 主要功能：
//! - 在临时目录中克隆仓库（子目录模式使用 sparse-checkout 优化）
//! - 将指定子目录或整个仓库复制到目标路径
//! - 自动清理临时文件，不污染当前项目的 .git 结构

use anyhow::{anyhow, bail, Context, Result};
use clap::Parser;
use std::path::{Path, PathBuf};
use std::process::Command;
use tempfile::TempDir;

/// 从 GitHub 仓库下载指定子目录或整个仓库到本地
#[derive(Parser, Debug)]
#[command(name = "git-get")]
#[command(author, version, about, long_about = None)]
struct Args {
    /// GitHub URL 或仓库标识
    /// 支持以下格式:
    /// 1. 完整 GitHub URL: https://github.com/owner/repo/tree/branch/path/to/dir
    /// 2. 简写: owner/repo
    /// 3. 完整 Git URL: https://github.com/owner/repo.git
    #[arg(short, long)]
    repo: Option<String>,

    /// 分支名（当使用简写格式时可指定，URL 格式时会自动提取）
    #[arg(short, long)]
    branch: Option<String>,

    /// 仓库内的子目录路径（可选，URL 格式时会自动提取）
    #[arg(short, long)]
    path: Option<String>,

    /// 本地目标目录路径（可选，默认使用 path 的最后一段或仓库名）
    #[arg(short, long)]
    dest: Option<String>,

    /// GitHub 访问 token（预留，用于私有仓库）
    #[arg(long)]
    token: Option<String>,

    /// GitHub URL（位置参数，可直接传入 URL 而不用 --repo）
    /// 例如: git-get https://github.com/owner/repo/tree/main/examples/servers
    #[arg(value_name = "URL")]
    url: Option<String>,
}

/// 从 GitHub URL 解析出的信息
#[derive(Debug)]
struct ParsedGitHubUrl {
    repo: String,
    branch: Option<String>,
    path: Option<String>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("❌ 错误: {:#}", e);
        std::process::exit(1);
    }
}

fn run() -> Result<()> {
    let args = Args::parse();

    // 解析输入，获取 repo、branch、path
    let (repo, branch, path) = parse_input(&args)?;

    // 决定目标路径（如果未提供，使用 path 的最后一段或仓库名）
    let dest = args.dest.unwrap_or_else(|| {
        if let Some(path) = path.as_deref() {
            path.split('/')
                .last()
                .unwrap_or("download")
                .to_string()
        } else {
            repo.split('/')
                .last()
                .unwrap_or("download")
                .trim_end_matches(".git")
                .to_string()
        }
    });

    // 验证并构建仓库 URL
    let repo_url = build_repo_url(&repo)?;
    println!("📦 仓库: {}", repo_url);
    println!("🌿 分支: {}", branch);
    if let Some(path) = path.as_deref() {
        println!("📁 子目录: {}", path);
    } else {
        println!("📁 子目录: <整个仓库>");
    }
    println!("📍 目标路径: {}", dest);

    // 检查目标路径安全性
    let dest_path = PathBuf::from(&dest);
    check_dest_path_safety(&dest_path, &dest)?;

    // 创建临时目录（作用域结束自动清理）
    let temp_dir = TempDir::new().context("无法创建临时目录")?;
    let temp_path = temp_dir.path();
    println!("🔧 临时目录: {}", temp_path.display());

    // 在临时目录中克隆仓库：有 path 时仅拉取子目录；无 path 时拉取整个仓库
    clone_repository(temp_path, &repo_url, &branch, path.as_deref(), args.token.as_deref())?;

    // 确定源路径
    let source_path = if let Some(path) = path.as_deref() {
        let source_path = temp_path.join(path);
        if !source_path.exists() {
            bail!(
                "远程仓库中未找到指定子目录: {}",
                path
            );
        }
        source_path
    } else {
        temp_path.to_path_buf()
    };

    // 复制子目录到目标路径
    copy_directory(&source_path, &dest_path)?;

    if path.is_some() {
        println!("✅ 完成! 子目录已复制到: {}", dest);
    } else {
        println!("✅ 完成! 仓库已复制到: {}", dest);
    }

    // 尝试添加到 .gitignore
    add_to_gitignore(&dest)?;

    // temp_dir 在此处被 drop，自动清理
    Ok(())
}

/// 解析用户输入，支持两种模式：
/// 1. URL 模式：从完整的 GitHub URL 中提取信息
/// 2. 分散参数模式：使用 --repo, --branch, --path 参数
fn parse_input(args: &Args) -> Result<(String, String, Option<String>)> {
    // 优先使用位置参数 URL
    let input_url = args.url.as_ref().or(args.repo.as_ref());

    if let Some(url) = input_url {
        // 尝试解析 GitHub URL
        if url.contains("github.com") && url.contains("/tree/") {
            let parsed = parse_github_url(url)?;
            
            let repo = parsed.repo;
            let branch = args.branch.clone()
                .or(parsed.branch)
                .unwrap_or_else(|| "main".to_string());
            let path = args.path.clone().or(parsed.path);
            
            return Ok((repo, branch, path));
        }
        
        // 否则作为 repo 参数处理
        let repo = url.clone();
        let branch = args.branch.clone().unwrap_or_else(|| "main".to_string());
        let path = args.path.clone();
        
        return Ok((repo, branch, path));
    }

    // 如果没有提供任何输入
    bail!("缺少输入！请提供 GitHub URL 或使用 --repo 参数\n\n使用示例:\n  git-get https://github.com/owner/repo/tree/main/path/to/dir\n  git-get --repo owner/repo --path path/to/dir");
}

/// 解析 GitHub URL，提取 repo、branch 和 path
/// 支持格式: https://github.com/owner/repo/tree/branch/path/to/dir
fn parse_github_url(url: &str) -> Result<ParsedGitHubUrl> {
    // 移除末尾的斜杠
    let url = url.trim_end_matches('/');
    
    // 检查是否包含 github.com
    if !url.contains("github.com") {
        bail!("不是有效的 GitHub URL: {}", url);
    }

    // 提取 github.com 后面的部分
    let parts: Vec<&str> = url.split("github.com/").collect();
    if parts.len() != 2 {
        bail!("无法解析 GitHub URL: {}", url);
    }

    let path_part = parts[1];
    let segments: Vec<&str> = path_part.split('/').collect();

    // 至少需要 owner/repo
    if segments.len() < 2 {
        bail!("URL 格式错误，无法提取仓库信息: {}", url);
    }

    let owner = segments[0];
    let repo_name = segments[1].trim_end_matches(".git");
    let repo = format!("{}/{}", owner, repo_name);

    // 检查是否包含 /tree/ 或 /blob/
    let mut branch = None;
    let mut path = None;

    if segments.len() > 2 {
        if segments[2] == "tree" || segments[2] == "blob" {
            if segments.len() > 3 {
                branch = Some(segments[3].to_string());
                
                // 如果有更多段，组合成路径
                if segments.len() > 4 {
                    path = Some(segments[4..].join("/"));
                }
            }
        }
    }

    Ok(ParsedGitHubUrl {
        repo,
        branch,
        path,
    })
}

/// 检查目标路径的安全性
/// 只允许不存在的路径或空目录，防止覆盖已有文件造成数据损失
fn check_dest_path_safety(dest_path: &Path, dest_str: &str) -> Result<()> {
    // 如果路径不存在，直接返回（安全）
    if !dest_path.exists() {
        return Ok(());
    }

    // 如果存在但不是目录，报错
    if !dest_path.is_dir() {
        bail!(
            "目标路径已存在且不是目录: {}",
            dest_str
        );
    }

    // 检查目录是否为空
    let entries = std::fs::read_dir(dest_path)
        .with_context(|| format!("无法读取目标目录: {}", dest_str))?;

    // 如果目录包含任何内容，报错
    if entries.count() > 0 {
        bail!(
            "目标目录已存在且不为空: {}\n提示: 为了安全起见，git-get 只能写入空目录或不存在的目录",
            dest_str
        );
    }

    // 目录存在但为空，安全
    Ok(())
}

/// 将 repo 参数转换为完整的 Git URL
fn build_repo_url(repo: &str) -> Result<String> {
    // 已经是完整 URL
    if repo.starts_with("https://") || repo.starts_with("git@") {
        return Ok(repo.to_string());
    }

    // owner/repo 格式
    let parts: Vec<&str> = repo.split('/').collect();
    if parts.len() == 2 && !parts[0].is_empty() && !parts[1].is_empty() {
        return Ok(format!("https://github.com/{}.git", repo));
    }

    Err(anyhow!(
        "无效的仓库格式: {}。支持格式: owner/repo 或 https://github.com/owner/repo.git",
        repo
    ))
}

/// 在临时目录中克隆仓库
/// - subdir 为 Some 时：使用 sparse-checkout 仅拉取指定子目录
/// - subdir 为 None 时：拉取整个仓库
fn clone_repository(
    temp_dir: &Path,
    repo_url: &str,
    branch: &str,
    subdir: Option<&str>,
    _token: Option<&str>,
) -> Result<()> {
    println!("📥 正在初始化仓库...");

    // 1. git init
    run_git_command(temp_dir, &["init"])?;

    // 2. git remote add origin <url>
    run_git_command(temp_dir, &["remote", "add", "origin", repo_url])?;

    if let Some(subdir) = subdir {
        // 3. 启用 sparse-checkout
        run_git_command(temp_dir, &["config", "core.sparseCheckout", "true"])?;

        // 4. 配置 sparse-checkout 路径
        let sparse_checkout_path = temp_dir.join(".git/info/sparse-checkout");
        std::fs::create_dir_all(sparse_checkout_path.parent().unwrap())?;
        std::fs::write(&sparse_checkout_path, format!("{}\n", subdir))
            .context("无法写入 sparse-checkout 配置")?;

        println!("📥 正在拉取仓库（仅获取指定子目录）...");
    } else {
        println!("📥 正在拉取仓库（完整仓库）...");
    }

    // 5. git fetch --depth=1 origin <branch>
    let fetch_result = run_git_command(temp_dir, &["fetch", "--depth=1", "origin", branch]);
    
    // 如果指定分支失败，尝试 master
    if fetch_result.is_err() && branch == "main" {
        println!("⚠️  分支 'main' 不存在，尝试 'master'...");
        run_git_command(temp_dir, &["fetch", "--depth=1", "origin", "master"])
            .context("无法拉取仓库，请检查仓库地址和分支名是否正确")?;
        run_git_command(temp_dir, &["checkout", "FETCH_HEAD"])?;
    } else {
        fetch_result.context("无法拉取仓库，请检查仓库地址和分支名是否正确")?;
        // 6. git checkout FETCH_HEAD
        run_git_command(temp_dir, &["checkout", "FETCH_HEAD"])?;
    }

    println!("📥 拉取完成");
    Ok(())
}

/// 执行 git 命令并检查结果
fn run_git_command(working_dir: &Path, args: &[&str]) -> Result<()> {
    let output = Command::new("git")
        .current_dir(working_dir)
        .args(args)
        .output()
        .with_context(|| format!("无法执行 git 命令: git {}", args.join(" ")))?;

    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "git {} 执行失败: {}",
            args.join(" "),
            stderr.trim()
        );
    }

    Ok(())
}

/// 递归复制目录，排除 .git 目录
fn copy_directory(src: &Path, dest: &Path) -> Result<()> {
    println!("📋 正在复制文件...");

    // 创建目标目录
    std::fs::create_dir_all(dest)
        .with_context(|| format!("无法创建目标目录: {}", dest.display()))?;

    copy_dir_recursive(src, dest)?;

    Ok(())
}

/// 递归复制目录内容，跳过 .git 目录
fn copy_dir_recursive(src: &Path, dest: &Path) -> Result<()> {
    for entry in std::fs::read_dir(src)
        .with_context(|| format!("无法读取目录: {}", src.display()))?
    {
        let entry = entry?;
        let file_name = entry.file_name();
        let file_name_str = file_name.to_string_lossy();

        // 跳过 .git 目录
        if file_name_str == ".git" {
            continue;
        }

        let src_path = entry.path();
        let dest_path = dest.join(&file_name);

        if src_path.is_dir() {
            std::fs::create_dir_all(&dest_path)?;
            copy_dir_recursive(&src_path, &dest_path)?;
        } else {
            std::fs::copy(&src_path, &dest_path)
                .with_context(|| format!("无法复制文件: {}", src_path.display()))?;
        }
    }

    Ok(())
}

/// 添加目标路径到 .gitignore 文件
/// 只有当 .gitignore 文件存在时才会添加
fn add_to_gitignore(dest_path: &str) -> Result<()> {
    let gitignore_path = PathBuf::from(".gitignore");
    
    // 检查 .gitignore 是否存在
    if !gitignore_path.exists() {
        // 不存在时静默返回，不做任何操作
        return Ok(());
    }

    // 读取现有内容
    let content = std::fs::read_to_string(&gitignore_path)
        .context("无法读取 .gitignore 文件")?;

    // 规范化路径（移除开头的 ./ 以保持一致性）
    let normalized_path = dest_path.trim_start_matches("./");

    // 检查是否已存在该条目
    for line in content.lines() {
        let trimmed = line.trim();
        // 跳过注释和空行
        if trimmed.starts_with('#') || trimmed.is_empty() {
            continue;
        }
        // 检查是否已存在（支持带 ./ 和不带 ./ 的格式）
        if trimmed == normalized_path || trimmed == format!("./{}", normalized_path) {
            // 已存在，不需要添加
            return Ok(());
        }
    }

    // 准备要添加的内容
    let mut new_content = content;
    
    // 如果文件不是以换行结束，先添加一个换行
    if !new_content.is_empty() && !new_content.ends_with('\n') {
        new_content.push('\n');
    }

    // 添加注释和路径
    new_content.push_str(&format!(
        "\n# Added by git-get\n{}\n",
        normalized_path
    ));

    // 写回文件
    std::fs::write(&gitignore_path, new_content)
        .context("无法写入 .gitignore 文件")?;

    println!("📝 已将 '{}' 添加到 .gitignore", normalized_path);

    Ok(())
}
