/// Bridge CLI filter args to core PackageFilters.
///
/// This lives in melos-cli (not melos-core) because GlobalFilterArgs is a
/// clap-derived type that belongs to the CLI layer. We use a free function
/// instead of `impl From` to satisfy the orphan rule (neither type is local).
use crate::cli::GlobalFilterArgs;
use melos_core::config::filter::PackageFilters;

pub fn package_filters_from_args(args: &GlobalFilterArgs) -> PackageFilters {
    PackageFilters {
        flutter: args.flutter_filter(),
        dir_exists: args.dir_exists.clone(),
        file_exists: args.file_exists.clone(),
        depends_on: if args.depends_on.is_empty() {
            None
        } else {
            Some(args.depends_on.clone())
        },
        no_depends_on: if args.no_depends_on.is_empty() {
            None
        } else {
            Some(args.no_depends_on.clone())
        },
        ignore: if args.ignore.is_empty() {
            None
        } else {
            Some(args.ignore.clone())
        },
        scope: if args.scope.is_empty() {
            None
        } else {
            Some(args.scope.clone())
        },
        no_private: args.no_private,
        diff: args.effective_diff().map(String::from),
        category: if args.category.is_empty() {
            None
        } else {
            Some(args.category.clone())
        },
        include_dependencies: args.include_dependencies,
        include_dependents: args.include_dependents,
        published: args.published_filter(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_global_filter_args() {
        let args = GlobalFilterArgs {
            scope: vec!["app*".to_string()],
            ignore: vec!["test*".to_string()],
            diff: Some("main".to_string()),
            since: None,
            dir_exists: Some("lib".to_string()),
            file_exists: None,
            flutter: true,
            no_flutter: false,
            depends_on: vec!["core".to_string()],
            no_depends_on: vec![],
            no_private: true,
            category: vec!["apps".to_string()],
            include_dependencies: true,
            include_dependents: false,
            published: false,
            no_published: false,
        };
        let filters = package_filters_from_args(&args);
        assert_eq!(filters.flutter, Some(true));
        assert_eq!(filters.scope, Some(vec!["app*".to_string()]));
        assert_eq!(filters.ignore, Some(vec!["test*".to_string()]));
        assert_eq!(filters.diff, Some("main".to_string()));
        assert_eq!(filters.dir_exists, Some("lib".to_string()));
        assert!(filters.file_exists.is_none());
        assert_eq!(filters.depends_on, Some(vec!["core".to_string()]));
        assert!(filters.no_depends_on.is_none());
        assert!(filters.no_private);
        assert_eq!(filters.category, Some(vec!["apps".to_string()]));
        assert!(filters.include_dependencies);
        assert!(!filters.include_dependents);
    }

    #[test]
    fn test_from_global_filter_args_no_flutter() {
        let args = GlobalFilterArgs {
            flutter: false,
            no_flutter: true,
            ..Default::default()
        };
        let filters = package_filters_from_args(&args);
        assert_eq!(filters.flutter, Some(false));
    }

    #[test]
    fn test_from_global_filter_args_since_alias() {
        let args = GlobalFilterArgs {
            since: Some("v1.0.0".to_string()),
            ..Default::default()
        };
        let filters = package_filters_from_args(&args);
        assert_eq!(filters.diff, Some("v1.0.0".to_string()));
    }

    #[test]
    fn test_from_global_filter_args_published() {
        let args = GlobalFilterArgs {
            published: true,
            ..Default::default()
        };
        let filters = package_filters_from_args(&args);
        assert_eq!(filters.published, Some(true));
    }

    #[test]
    fn test_from_global_filter_args_no_published() {
        let args = GlobalFilterArgs {
            no_published: true,
            ..Default::default()
        };
        let filters = package_filters_from_args(&args);
        assert_eq!(filters.published, Some(false));
    }
}
