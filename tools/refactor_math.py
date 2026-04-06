import os
import re

pairs = [
    (r'gizmo_math::vec2::Vec2', r'gizmo_math::Vec2'),
    (r'gizmo_math::vec3::Vec3', r'gizmo_math::Vec3'),
    (r'gizmo_math::vec4::Vec4', r'gizmo_math::Vec4'),
    (r'gizmo_math::mat4::Mat4', r'gizmo_math::Mat4'),
    (r'gizmo_math::quat::Quat', r'gizmo_math::Quat'),
    (r'Mat4::perspective\(', r'Mat4::perspective_rh_zo('),
    (r'Mat4::orthographic\(', r'Mat4::orthographic_rh_zo('),
    (r'Mat4::translation\(', r'Mat4::from_translation('),
    (r'Mat4::scale\(', r'Mat4::from_scale('),
    (r'Mat4::rotation_y\(', r'Mat4::from_rotation_y('),
]

for root, dirs, files in os.walk('crates'):
    for file in files:
        if file.endswith('.rs'):
            path = os.path.join(root, file)
            with open(path, 'r') as f:
                content = f.read()
            original = content
            for p, repl in pairs:
                content = re.sub(p, repl, content)
            if original != content:
                with open(path, 'w') as f:
                    f.write(content)
                print(f'Updated {path}')
