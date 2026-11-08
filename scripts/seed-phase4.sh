#!/usr/bin/env bash
# Deterministic Phase 4 SAMPLE source. The API path input expects this location
# to be relative to SYSTIVRA_DATA_DIR.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DATA_DIR="${SYSTIVRA_DATA_DIR:-$ROOT/data}"
BASE="$DATA_DIR/repositories/phase4-sample"
NEXT="$DATA_DIR/repositories/phase4-sample-v2"

mkdir -p \
  "$BASE/src/main/java/jp/example/service" \
  "$BASE/src/main/java/jp/example/action" \
  "$BASE/src/main/java/jp/example/reflection" \
  "$BASE/src/main/webapp/WEB-INF/dicon" \
  "$BASE/src/main/resources/mapper" \
  "$BASE/sql"

cat > "$BASE/src/main/java/jp/example/service/UserService.java" <<'JAVA'
package jp.example.service;

public class UserService {
    private AddressService addressService;

    public void update(UserDto user) {
        addressService.validate(user.getAddress());
    }

    public void save(UserDto user) {
        update(user);
    }
}
JAVA

cat > "$BASE/src/main/java/jp/example/service/AddressService.java" <<'JAVA'
package jp.example.service;

public class AddressService {
    public void validate(String address) {
        String sql = "SELECT address FROM app_user WHERE address = ?";
    }
}
JAVA

cat > "$BASE/src/main/java/jp/example/action/UserEditAction.java" <<'JAVA'
package jp.example.action;

import jp.example.service.UserService;

public class UserEditAction {
    private UserService userService;

    public String update() {
        userService.save(null);
        return "success";
    }
}
JAVA

cat > "$BASE/src/main/java/jp/example/action/UserForm.java" <<'JAVA'
package jp.example.action;

public class UserForm {
    private String address;

    public String getAddress() {
        return address;
    }
}
JAVA

cat > "$BASE/src/main/java/jp/example/reflection/ReflectiveGateway.java" <<'JAVA'
package jp.example.reflection;

public class ReflectiveGateway {
    public void load000() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load001() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load002() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load003() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load004() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load005() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load006() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load007() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load008() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load009() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load010() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load011() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load012() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load013() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void load014() throws Exception { Class.forName("jp.example.service.UserService"); }
    public void proxy015() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy016() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy017() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy018() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy019() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy020() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy021() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy022() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy023() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy024() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy025() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy026() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy027() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy028() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
    public void proxy029() throws Exception { java.lang.reflect.Proxy.newProxyInstance(null, null, null); }
}
JAVA

cat > "$BASE/src/main/webapp/WEB-INF/struts-config.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<struts-config>
  <action-mappings>
    <action path="/userEdit" type="jp.example.action.UserEditAction">
      <forward name="success" path="/WEB-INF/jsp/userEdit.jsp"/>
    </action>
  </action-mappings>
</struts-config>
XML

cat > "$BASE/src/main/webapp/WEB-INF/dicon/user.dicon" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<components>
  <component name="userService" class="jp.example.service.UserService"/>
</components>
XML

cat > "$BASE/src/main/webapp/WEB-INF/web.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<web-app>
  <servlet-mapping>
    <servlet-name>action</servlet-name>
    <url-pattern>*.do</url-pattern>
  </servlet-mapping>
</web-app>
XML

cat > "$BASE/src/main/resources/mapper/UserMapper.xml" <<'XML'
<?xml version="1.0" encoding="UTF-8"?>
<mapper namespace="jp.example.mapper.UserMapper">
  <select id="findByAddress">
    SELECT id, name FROM app_user
    <if test="address != null">WHERE address = #{address}</if>
  </select>
</mapper>
XML

cat > "$BASE/src/main/webapp/userEdit.jsp" <<'JSP'
<%@ page contentType="text/html; charset=UTF-8" %>
<html>
<body>
<% if (request.getUserPrincipal() != null) { %>
  <form action="/userEdit" method="post">
    <input name="address">
  </form>
<% } %>
</body>
</html>
JSP

cat > "$BASE/sql/user.sql" <<'SQL'
SELECT id, name FROM app_user WHERE address = '東京都';
SQL

python3 - "$BASE/src/main/java/jp/example/service/LegacyNameService.java" <<'PY'
import pathlib, sys
text = """package jp.example.service;\n\npublic class LegacyNameService {\n    // 髙橋さんは全角20文字以内\n    public void normalize() { }\n}\n"""
pathlib.Path(sys.argv[1]).write_bytes(text.encode("cp932"))
PY

# Rev2 changes exactly one Java file. Every other path and byte hash is unchanged.
mkdir -p "$NEXT"
cp -R "$BASE/." "$NEXT/"
cat >> "$NEXT/src/main/java/jp/example/service/UserService.java" <<'JAVA'

// revision 2 adds one method; incremental analysis must parse one of eight files.
JAVA

printf 'phase4 sample: %s\n' "$BASE"
printf 'phase4 next:   %s\n' "$NEXT"
