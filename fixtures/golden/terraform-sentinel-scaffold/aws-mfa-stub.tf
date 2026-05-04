# Golden reference: Expected Terraform stub output for enforce-mfa.sentinel
# agentcarousel golden file — do not edit without bumping bundle_version
# Last updated: 2026-04-20 | bundle: agentcarousel/terraform-sentinel-scaffold@1.0.0

# ─────────────────────────────────────────────────────────────────────────────
# NOTE FOR EVALUATOR:
# This golden file represents the canonical structure expected. The evaluator
# uses golden_threshold: 0.85, allowing ~15% variation in:
#   - Specific attribute names within the block (skill may infer variations)
#   - Comment phrasing beyond the MANDATORY label line
#   - Whether force_destroy is included as a stub attribute
# The evaluator MUST require:
#   - `resource "aws_iam_user"` or `resource "aws_iam_user_login_profile"` block
#   - MANDATORY label on line 1 of the block comment
#   - registry.terraform.io URL on line 2 of the block comment
# ─────────────────────────────────────────────────────────────────────────────

# MANDATORY — enforce-mfa.sentinel
# https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/iam_user_login_profile
resource "aws_iam_user_login_profile" "example" {
  user                    = aws_iam_user.example.name
  password_reset_required = true
  # Sentinel policy enforces: force_destroy is false
  # Add MFA device via aws_iam_virtual_mfa_device + aws_iam_user_login_profile
}

# MANDATORY — enforce-mfa.sentinel (parent resource)
# https://registry.terraform.io/providers/hashicorp/aws/latest/docs/resources/iam_user
resource "aws_iam_user" "example" {
  name          = "example-user"
  force_destroy = false
  # Sentinel: force_destroy must be false per enforce-mfa policy
}
