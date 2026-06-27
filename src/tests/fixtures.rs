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
    fs::write(
        root.join("pkg/relative.py"),
        r#"
from .models import User

def relative_user(user_id, email):
    return User(user_id, email)
"#,
    )
    .expect("relative");
    fs::create_dir_all(root.join("src")).expect("src dir");
    fs::write(
        root.join("src/lib.rs"),
        r#"
use crate::service::Authenticator;

pub struct RustUser {
    pub user_id: String,
}

pub fn login_user(user_id: &str) -> RustUser {
    if user_id.is_empty() {
        panic!("missing user_id");
    }
    Authenticator::new().login(user_id)
}
"#,
    )
    .expect("rust lib");
    fs::write(
        root.join("src/service.rs"),
        r#"
pub struct Authenticator;

impl Authenticator {
    pub fn new() -> Self {
        Self
    }

    pub fn login(&self, user_id: &str) -> crate::RustUser {
        crate::RustUser {
            user_id: user_id.to_string(),
        }
    }
}
"#,
    )
    .expect("rust service");
    fs::create_dir_all(root.join("app")).expect("app dir");
    fs::write(
        root.join("app/Controller.cs"),
        r#"
using App.Services;

namespace App.Controllers;

public class LoginController {
    private readonly AuthService service = new AuthService();

    public UserDto Login(string userId) {
        if (string.IsNullOrEmpty(userId)) {
            throw new ArgumentException("missing userId");
        }
        return service.Login(userId);
    }
}
"#,
    )
    .expect("csharp controller");
    fs::write(
        root.join("app/AuthService.cs"),
        r#"
namespace App.Services;

public record UserDto(string UserId);

public class AuthService {
    public UserDto Login(string userId) {
        return new UserDto(userId);
    }
}
"#,
    )
    .expect("csharp service");
    fs::create_dir_all(root.join("web")).expect("web dir");
    fs::write(
        root.join("web/auth.ts"),
        r#"
import { AuditLogger } from "./logger";

export interface Session {
    userId: string;
}

export function loginUser(userId: string): Session {
    if (!userId) {
        throw new Error("missing userId");
    }
    AuditLogger.record(userId);
    return { userId };
}
"#,
    )
    .expect("typescript auth");
    fs::write(
        root.join("web/logger.tsx"),
        r#"
export class AuditLogger {
    static record(userId: string): void {
        console.log(userId);
    }
}

export const LoginView = () => <button>Login</button>;
"#,
    )
    .expect("tsx logger");
    fs::write(
        root.join("web/auth.js"),
        r#"
const { formatUser } = require("./format");

export function loginBrowser(userId) {
    if (!userId) {
        throw new Error("missing userId");
    }
    return formatUser(userId);
}
"#,
    )
    .expect("javascript auth");
    fs::write(
        root.join("web/format.jsx"),
        r#"
export class Formatter {
    render(userId) {
        return <span>{userId}</span>;
    }
}

export const formatUser = (userId) => ({ userId });
"#,
    )
    .expect("jsx format");
    fs::write(
        root.join("web/legacy.cjs"),
        r#"
const { formatUser } = require("./format");

function legacyLogin(userId) {
    return formatUser(userId);
}

module.exports = { legacyLogin };
"#,
    )
    .expect("cjs legacy");
    fs::write(
        root.join("web/browser.mjs"),
        r#"
import { formatUser } from "./format.jsx";

export function browserLogin(userId) {
    return formatUser(userId);
}
"#,
    )
    .expect("mjs browser");
    dir
}
