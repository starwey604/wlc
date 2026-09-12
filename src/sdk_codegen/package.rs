// SPDX-License-Identifier: Apache-2.0
use super::{BINDING_API_VERSION, plan::Plan};
use std::collections::BTreeMap;

pub(super) fn emit(plan: &Plan<'_>, files: &mut BTreeMap<String, String>) {
    let name = &plan.options.name;
    let distribution = format!("wirelink-{}", plan.package.replace('_', "-"));
    let upper = name.to_uppercase();
    let api = BINDING_API_VERSION.to_string();
    let values = [
        ("NAME", name.as_str()),
        ("PY_PACKAGE", plan.package.as_str()),
        ("CMAKE_PACKAGE", plan.cmake_package.as_str()),
        ("CMAKE_VERSION", plan.cmake_version.as_str()),
        ("VERSION", plan.options.package_version.as_str()),
        ("UPPER", upper.as_str()),
        ("DISTRIBUTION", distribution.as_str()),
        ("BINDING_API", api.as_str()),
    ];
    for (path, template) in [
        (
            "CMakeLists.txt".to_owned(),
            include_str!("templates/CMakeLists.txt.in"),
        ),
        (
            format!("cmake/{}Config.cmake.in", plan.cmake_package),
            include_str!("templates/Config.cmake.in"),
        ),
        (
            "pyproject.toml".to_owned(),
            include_str!("templates/pyproject.toml.in"),
        ),
        (
            "README.md".to_owned(),
            include_str!("templates/README.md.in"),
        ),
        (
            "wlc-sdk-info.json".to_owned(),
            "{\n  \"format\": \"wirelink-sdk-v1\",\n  \"binding_api\": @BINDING_API@,\n  \"namespace\": \"@NAME@\",\n  \"python_package\": \"@PY_PACKAGE@\",\n  \"distribution\": \"@DISTRIBUTION@\",\n  \"version\": \"@VERSION@\"\n}\n",
        ),
    ] {
        files.insert(path, crate::template::render(template, &values));
    }
    for (path, contents) in [
        ("LICENSE", include_str!("templates/LICENSE")),
        (
            "licenses/wirelink.txt",
            include_str!("templates/licenses/wirelink.txt"),
        ),
        (
            "licenses/asio.txt",
            include_str!("templates/licenses/asio.txt"),
        ),
        (
            "licenses/nanobind.txt",
            include_str!("templates/licenses/nanobind.txt"),
        ),
        (
            "licenses/robin_map.txt",
            include_str!("templates/licenses/robin_map.txt"),
        ),
    ] {
        files.insert(path.into(), contents.into());
    }
}
