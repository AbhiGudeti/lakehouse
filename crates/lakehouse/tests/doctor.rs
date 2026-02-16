//use assert_cmd::assert::Assert;
use assert_cmd::cargo::cargo_bin_cmd;

#[test]
fn doctor_prints_ok() {
    let mut cmd = cargo_bin_cmd!("lakehouse");
    cmd.arg("doctor");
    cmd.assert().success().stdout("Ok\n");
}
