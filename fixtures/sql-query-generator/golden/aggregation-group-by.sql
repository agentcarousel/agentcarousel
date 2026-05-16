SELECT
    u.name,
    COUNT(o.id) AS order_count
FROM users u
JOIN orders o ON o.user_id = u.id
WHERE o.created_at >= NOW() - INTERVAL '30 days'
GROUP BY u.id, u.name
ORDER BY order_count DESC;
