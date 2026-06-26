use std::fs;

use tempfile::TempDir;

pub fn fixture() -> TempDir {
    let dir = tempfile::tempdir().expect("tempdir");
    let root = dir.path();
    fs::create_dir_all(root.join("pkg")).expect("pkg dir");
    fs::write(
        root.join("pkg/core.py"),
        r#"
import time
from pkg.models import User

class Service:
    def login(self, user_id, email):
        if not user_id:
            raise ValueError("missing user_id")
        for item in range(3):
            time.sleep(0.01)
        return User(user_id, email)

def authenticate(user_id, email):
    try:
        svc = Service()
        return svc.login(user_id, email)
    except ValueError as err:
        raise RuntimeError("login failed") from err
"#,
    )
    .expect("core");
    fs::write(
        root.join("pkg/models.py"),
        r#"
class User:
    def __init__(self, user_id, email):
        self.user_id = user_id
        self.email = email
"#,
    )
    .expect("models");
    fs::write(
        root.join("pkg/api.py"),
        r#"
from pkg.core import authenticate

def login_endpoint(request):
    user_id = request["user_id"]
    email = request["email"]
    return authenticate(user_id, email)
"#,
    )
    .expect("api");
    dir
}
